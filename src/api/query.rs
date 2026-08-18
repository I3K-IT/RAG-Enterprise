//! Query endpoints:
//! POST /api/query         → { answer, sources }
//! POST /api/query/stream  → SSE  { token } … { done, sources }
//! GET  /api/chat/history  → { messages }
//! DELETE /api/chat/history → { deleted }

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    Json,
};
use futures_util::stream::unfold;
use serde::Deserialize;
use serde_json::json;

use crate::auth::jwt::Claims;
use crate::bench;
use crate::db;
use crate::rag::{prompt, retrieval, sources::Source};
use crate::state::AppState;

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct QueryRequest {
    pub query: String,
    #[serde(default)]
    pub top_k: Option<u64>,
    #[serde(default)]
    pub use_history: bool,
    /// SQLite conversation ID. When present, messages are stored in that
    /// conversation and history is read only from it.
    pub conversation_id: Option<String>,
}

fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": msg.to_string() }))).into_response()
}

/// eullm has been evicted from VRAM, or is about to be, because of
/// `unload_during_ingestion` — see AppState::ingestion_blocks_queries.
fn ingestion_busy_response() -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "a document is being ingested: the model is temporarily unloaded from VRAM, try again in a few seconds",
    )
}

// ── Shared setup: embed → search → load history → build prompt ────────────────

/// Per-stage timings from prepare(), used by query_stream() to record an
/// InferenceResult when --bench-live is enabled (see bench::LiveRecorder).
/// The non-streaming query() currently ignores them: it is not instrumented,
/// because the streaming path is the one the frontend actually uses (see the
/// note in query_stream).
pub(crate) struct PrepareTimings {
    pub(crate) embed_query: Duration,
    pub(crate) search: Duration,
    pub(crate) prompt_build: Duration,
}

async fn prepare(
    state: &AppState,
    question: &str,
    user_id: i64,
    use_history: bool,
    conversation_id: Option<&str>,
) -> anyhow::Result<(String, Vec<Source>, PrepareTimings)> {
    // 1. Embed the query (CPU/GPU bound). With ingestion_embedding=CandleGpu
    // (or =Eullm, where bge-m3 never touches the GPU at all) bge-m3 runs on
    // CPU here — a single short text, so the cost is acceptable — because
    // the GPU is reserved for eullm outside the ingestion window.
    let t = Instant::now();
    let svc = state.embeddings.clone();
    let q = question.to_owned();
    let query_vec = tokio::task::spawn_blocking(move || {
        let guard = svc.read().map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
        guard.embed_text(&q)
    })
    .await??;
    let embed_query = t.elapsed();

    // 2. Vector search (top_k=15, threshold=0.30 — MAPPA §5)
    let t = Instant::now();
    let hits = state
        .qdrant
        .search(query_vec, retrieval::TOP_K, Some(retrieval::RELEVANCE_THRESHOLD))
        .await?;
    let search = t.elapsed();

    // 3. Build sources and context string (not timed: the string join is
    // negligible, same convention as bench::run_inference)
    let sources: Vec<Source> = hits
        .iter()
        .map(|h| Source {
            document_id: h.payload.document_id.clone(),
            filename: h.payload.filename.clone(),
            chunk_index: h.payload.chunk_index,
            similarity: h.similarity,
            text: h.payload.text.clone(),
        })
        .collect();

    let context: String = sources
        .iter()
        .map(|s| format!("[{}]\n{}", s.filename, s.text))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    // 4. Load the last 3 exchanges from history if requested, then build the
    // prompt — a single "prompt_build" stage, which includes the SQLite
    // history query that bench::run_inference does not have, since history is
    // always empty there.
    let t = Instant::now();
    let history = if use_history {
        build_history_pairs(state, user_id, conversation_id).await?
    } else {
        vec![]
    };
    let full_prompt = prompt::build_prompt(&context, question, &history);
    let prompt_build = t.elapsed();

    tracing::info!(
        chars = full_prompt.len(),
        chunks = sources.len(),
        history_pairs = history.len(),
        "prompt costruito"
    );
    Ok((full_prompt, sources, PrepareTimings { embed_query, search, prompt_build }))
}

async fn build_history_pairs(
    state: &AppState,
    user_id: i64,
    conversation_id: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let msgs = if let Some(cid) = conversation_id {
        // Take the last 6 messages of this conversation
        db::conversations::list_by_conv_for_history(&state.db, cid, user_id, 6).await?
    } else {
        db::conversations::list_by_user(&state.db, user_id, 6).await?
    };
    // list returns DESC; reverse to chronological order.
    let asc: Vec<_> = msgs.into_iter().rev().collect();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i + 1 < asc.len() {
        if asc[i].role == "user" && asc[i + 1].role == "assistant" {
            pairs.push((asc[i].content.clone(), asc[i + 1].content.clone()));
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(pairs)
}

// ── POST /api/query ───────────────────────────────────────────────────────────

pub async fn query(
    State(state): State<AppState>,
    claims: Claims,
    Json(req): Json<QueryRequest>,
) -> Response {
    if state.ingestion_blocks_queries() {
        return ingestion_busy_response();
    }
    let conv_id = req.conversation_id.as_deref();
    // _timings: not instrumented — the frontend uses /api/query/stream (see
    // query_stream), which is where --bench-live records real queries.
    let (full_prompt, sources, _timings) =
        match prepare(&state, &req.query, claims.user_id, req.use_history, conv_id).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };

    let answer = match state.eullm.invoke(&full_prompt).await {
        Ok(a) => a,
        Err(e) => return err(StatusCode::BAD_GATEWAY, format!("eullm: {e}")),
    };

    // Empty answer — for instance the model produced only a <think> block that
    // was then stripped. Do NOT persist it: an empty "assistant" turn in the
    // history would contaminate the prompts of later attempts when
    // use_history=true.
    if answer.trim().is_empty() {
        tracing::warn!(query = %req.query, "eullm: empty answer");
        return err(StatusCode::BAD_GATEWAY, "the model produced no answer, please try again");
    }

    // Persist conversation
    let sources_json = serde_json::to_string(&sources).unwrap_or_default();
    let _ = db::conversations::insert(&state.db, claims.user_id, "user", &req.query, None, conv_id).await;
    let _ = db::conversations::insert(
        &state.db,
        claims.user_id,
        "assistant",
        &answer,
        Some(&sources_json),
        conv_id,
    )
    .await;

    Json(json!({
        "answer":  answer,
        "sources": sources,
    }))
    .into_response()
}

// ── POST /api/query/stream ────────────────────────────────────────────────────

/// State threaded through the unfold that produces the SSE stream. A struct
/// rather than a tuple because it accumulated too many fields. The
/// non-streaming query() does not need it (see above): only this path measures
/// TTFT and decode for --bench-live, since this is the one the frontend
/// uses.
struct StreamState {
    rx: tokio::sync::mpsc::Receiver<String>,
    acc: String,
    sources: Vec<Source>,
    db: sqlx::SqlitePool,
    uid: i64,
    cid: Option<String>,
    done: bool,
    gen_start: Instant,
    ttft: Option<Duration>,
    tokens: usize,
    live_bench: Option<Arc<bench::LiveRecorder>>,
    timings: PrepareTimings,
    chunks_retrieved: usize,
    query_text: String,
}

pub async fn query_stream(
    State(state): State<AppState>,
    claims: Claims,
    Json(req): Json<QueryRequest>,
) -> Response {
    if state.ingestion_blocks_queries() {
        return ingestion_busy_response();
    }
    let conv_id = req.conversation_id.as_deref();
    // Run setup synchronously before opening the SSE stream so we can return
    // a proper HTTP error if embed/search fails.
    let (full_prompt, sources, timings) =
        match prepare(&state, &req.query, claims.user_id, req.use_history, conv_id).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
    let chunks_retrieved = sources.len();

    // Persist user question (answer is stored when the stream finishes).
    let _ = db::conversations::insert(
        &state.db, claims.user_id, "user", &req.query, None,
        req.conversation_id.as_deref(),
    )
    .await;

    // Start eullm streaming in background.
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
    let eullm = state.eullm.clone();
    let prompt_clone = full_prompt.clone();
    tokio::spawn(async move {
        if let Err(e) = eullm.invoke_stream(&prompt_clone, tx).await {
            tracing::error!("eullm stream error: {e}");
        }
    });

    let stream_state = StreamState {
        rx,
        acc: String::new(),
        sources,
        db: state.db.clone(),
        uid: claims.user_id,
        cid: req.conversation_id.clone(),
        done: false,
        gen_start: Instant::now(),
        ttft: None,
        tokens: 0,
        live_bench: state.live_bench.clone(),
        timings,
        chunks_retrieved,
        query_text: req.query.clone(),
    };

    // Convert mpsc receiver into an SSE stream.
    let stream = unfold(stream_state, |mut s| async move {
        if s.done {
            return None;
        }
        match s.rx.recv().await {
            Some(token) => {
                if s.ttft.is_none() {
                    s.ttft = Some(s.gen_start.elapsed());
                }
                s.tokens += 1;
                s.acc.push_str(&token);
                let ev = Event::default().data(json!({ "token": token }).to_string());
                Some((Ok::<_, Infallible>(ev), s))
            }
            None => {
                // Channel closed — persist the assistant reply, if non-empty
                // (see the non-streaming query() for why), and emit the final
                // event.
                let total_generation = s.gen_start.elapsed();
                if !s.acc.trim().is_empty() {
                    let sources_json = serde_json::to_string(&s.sources).unwrap_or_default();
                    let _ = db::conversations::insert(
                        &s.db, s.uid, "assistant", &s.acc, Some(&sources_json),
                        s.cid.as_deref(),
                    )
                    .await;
                } else {
                    tracing::warn!("eullm stream: empty answer, not persisted");
                }

                if let Some(rec) = &s.live_bench {
                    rec.record_inference(bench::InferenceResult {
                        query: s.query_text.clone(),
                        embed_query: s.timings.embed_query,
                        search: s.timings.search,
                        prompt_build: s.timings.prompt_build,
                        ttft: s.ttft.unwrap_or(total_generation),
                        total_generation,
                        tokens_generated: s.tokens,
                        chunks_retrieved: s.chunks_retrieved,
                        // Not applicable outside --bench <file>: there it
                        // compares the retrieved chunks against the single
                        // just-ingested document, whereas the real collection
                        // holds many.
                        chunks_from_bench_doc: 0,
                    });
                }

                let sources_for_event = std::mem::take(&mut s.sources);
                let ev = Event::default()
                    .data(json!({ "done": true, "sources": sources_for_event }).to_string());
                s.done = true;
                Some((Ok::<_, Infallible>(ev), s))
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

// ── GET /api/chat/history ─────────────────────────────────────────────────────

pub async fn chat_history(State(state): State<AppState>, claims: Claims) -> Response {
    match db::conversations::list_by_user(
        &state.db,
        claims.user_id,
        db::conversations::MAX_MESSAGES_PER_USER,
    )
    .await
    {
        Ok(msgs) => Json(json!({ "messages": msgs })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ── DELETE /api/chat/history ──────────────────────────────────────────────────

pub async fn delete_chat_history(State(state): State<AppState>, claims: Claims) -> Response {
    match db::conversations::delete_by_user(&state.db, claims.user_id).await {
        Ok(n) => Json(json!({ "deleted": n })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
