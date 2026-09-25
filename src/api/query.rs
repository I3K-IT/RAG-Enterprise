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
use crate::clients::eullm::StreamItem;
use crate::db;
use crate::rag::{prompt, retrieval, sources::Source, vector_store::ChunkPayload};
use crate::state::AppState;

/// Longest question this API will answer.
///
/// The only limit before was axum's 2 MB default body: a question that size
/// was tokenised in full, embedded on the CPU, and pasted into the prompt,
/// so any authenticated account — including a plain `user`, who cannot
/// upload anything — could spend minutes of CPU and gigabytes of memory per
/// request, as often as it liked. Four thousand characters is roughly two
/// pages of prose, far past any real question, and the rejection is cheap
/// because it happens before the embedding.
pub const MAX_QUERY_CHARS: usize = 4_000;

/// Upper bound for the client-supplied `top_k` (see `resolve_top_k`).
/// Fifteen chunks already cover several documents; beyond fifty the prompt
/// only grows while relevance dilutes, and every extra chunk costs
/// embedding-adjacent work per query on cards where that is scarcest.
pub const MAX_TOP_K: u64 = 50;

/// The retrieval depth for one question. Absent means the default; present
/// is clamped, not rejected, so no client that worked before can break —
/// `top_k: 0` retrieves a single chunk instead of erroring, and an absurd
/// value stops at the ceiling instead of fanning out to Qdrant unbounded.
/// A free function so the rule can be tested without a running server,
/// like `validate_query` below.
pub(crate) fn resolve_top_k(requested: Option<u64>) -> u64 {
    requested.unwrap_or(retrieval::TOP_K).clamp(1, MAX_TOP_K)
}

/// `Err(message)` when the question is empty or too long. A free function so
/// the rule can be tested without a running server, like `validate_auth` and
/// `validate_new_password` elsewhere in this codebase.
pub(crate) fn validate_query(question: &str) -> Result<(), String> {
    if question.trim().is_empty() {
        return Err("the question is empty".to_owned());
    }
    // Characters, not bytes: an accented question must not count double
    // against a limit expressed to the user in characters.
    let len = question.chars().count();
    if len > MAX_QUERY_CHARS {
        return Err(format!(
            "the question is {len} characters long; the maximum is {MAX_QUERY_CHARS}"
        ));
    }
    Ok(())
}

/// Text to feed the answering LLM for one retrieved chunk: the enriched
/// `retrieval_text` when present, otherwise the chunk's own `text`. Never
/// the reverse — see `ChunkPayload::retrieval_text`'s doc comment.
fn context_text(payload: &ChunkPayload) -> &str {
    payload.retrieval_text.as_deref().unwrap_or(&payload.text)
}

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

/// A 4xx tells the caller what they got wrong, so its message travels.
/// A 5xx does not: `msg` is an anyhow chain carrying whatever context the
/// failure picked up on the way out — filesystem paths, the Qdrant URL, SQL
/// text, the body of an eullm reply — and handing that to an unauthenticated
/// caller is free reconnaissance. The detail goes to the log, where it is
/// actually useful, and the response says only that something broke.
fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    if status.is_server_error() {
        tracing::error!(status = %status, detail = %msg, "request failed");
        return (status, Json(json!({ "error": "internal server error" }))).into_response();
    }
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
    top_k: u64,
) -> anyhow::Result<(String, Vec<Source>, PrepareTimings)> {
    // 1. Embed the query. With ingestion_embedding=Eullm this now goes
    // through eullm's own POST /api/embed (an HTTP call, .await directly) —
    // the same on-demand coexist-or-evict eullm already does for ingestion
    // (config::IngestionEmbedding::Eullm), extended to query time too.
    // Otherwise Candle, CPU/GPU-bound via spawn_blocking; with
    // ingestion_embedding=CandleGpu bge-m3 runs on CPU here regardless (a
    // single short text, so the cost is acceptable) because the GPU is
    // reserved for eullm outside the ingestion window.
    //
    // Caveat worth knowing before relying on this on a tight-VRAM card: if
    // bge-m3 and the chat model do not both fit, every query now pays a
    // potential double swap — eullm may evict the chat model to embed the
    // question, then evict bge-m3 right back out to answer it. Harmless
    // when both fit together (the common case eullm's coexist logic is
    // built for); adds real per-query latency when they do not.
    let t = Instant::now();
    let query_vec = if state.settings.embeddings.ingestion_embedding
        == crate::config::IngestionEmbedding::Eullm
    {
        state
            .eullm
            .embed_texts(crate::config::EULLM_EMBEDDING_MODEL, &[question], None)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("eullm embed: empty response for a single-text request"))?
    } else {
        let svc = state
            .embeddings
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Candle embeddings not loaded (ingestion_embedding=eullm?)"))?;
        let q = question.to_owned();
        tokio::task::spawn_blocking(move || {
            let guard = svc.read().map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
            guard.embed_text(&q)
        })
        .await??
    };
    let embed_query = t.elapsed();

    // 2. Vector search (threshold=0.30 — MAPPA §5; depth is the caller's
    // resolved top_k, TOP_K when the request carries none).
    let t = Instant::now();
    let hits = state
        .qdrant
        .search(query_vec, top_k, Some(retrieval::RELEVANCE_THRESHOLD))
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
            source_start_byte: h.payload.source_start_byte,
            source_end_byte: h.payload.source_end_byte,
            page_start: h.payload.page_start,
            page_end: h.payload.page_end,
            provenance_id: h.payload.provenance_id.clone(),
        })
        .collect();

    // context_text (not sources[i].text, which is always the original
    // chunk — see ChunkPayload::retrieval_text): the answering LLM benefits
    // from the same enrichment the embedding was computed on, even though
    // that enrichment must never be shown to the user as if it were the
    // source itself.
    let context: String = hits
        .iter()
        .map(|h| format!("[{}]\n{}", h.payload.filename, context_text(&h.payload)))
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
        "prompt built"
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

/// Rejects a `conversation_id` the caller does not own, before any work is
/// done on the request.
///
/// The id arrives in the request body, so it is the client's word for which
/// conversation this is. Reads have always been safe — every statement in
/// db::conversations filters by user_id too — but the write paths below file
/// the question and the answer under whatever id came in, and inserting under
/// someone else's conversation bumps it to the top of their list. `None` (no
/// conversation) is legitimate and passes through.
async fn check_conversation_owned(
    state: &AppState,
    conv_id: Option<&str>,
    user_id: i64,
) -> Result<(), Response> {
    let Some(cid) = conv_id else { return Ok(()) };
    match db::conversations::is_owned_by(&state.db, cid, user_id).await {
        // 404, not 403: whether someone else's conversation exists is not
        // something to confirm to this user.
        Ok(true) => Ok(()),
        Ok(false) => Err(err(StatusCode::NOT_FOUND, "conversation not found")),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
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
    if let Err(msg) = validate_query(&req.query) {
        return err(StatusCode::BAD_REQUEST, msg);
    }
    let conv_id = req.conversation_id.as_deref();
    if let Err(resp) = check_conversation_owned(&state, conv_id, claims.user_id).await {
        return resp;
    }
    // _timings: not instrumented — the frontend uses /api/query/stream (see
    // query_stream), which is where --bench-live records real queries.
    let top_k = resolve_top_k(req.top_k);
    let (full_prompt, sources, _timings) = match prepare(
        &state,
        &req.query,
        claims.user_id,
        req.use_history,
        conv_id,
        top_k,
    )
    .await
    {
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
    let sources_json = crate::rag::sources::history_json(&sources);
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
    rx: tokio::sync::mpsc::Receiver<StreamItem>,
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
    if let Err(msg) = validate_query(&req.query) {
        return err(StatusCode::BAD_REQUEST, msg);
    }
    let conv_id = req.conversation_id.as_deref();
    if let Err(resp) = check_conversation_owned(&state, conv_id, claims.user_id).await {
        return resp;
    }
    // Run setup synchronously before opening the SSE stream so we can return
    // a proper HTTP error if embed/search fails.
    let top_k = resolve_top_k(req.top_k);
    let (full_prompt, sources, timings) = match prepare(
        &state,
        &req.query,
        claims.user_id,
        req.use_history,
        conv_id,
        top_k,
    )
    .await
    {
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
    let (tx, rx) = tokio::sync::mpsc::channel::<StreamItem>(64);
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
            Some(StreamItem::Token(token)) => {
                if s.ttft.is_none() {
                    s.ttft = Some(s.gen_start.elapsed());
                }
                s.tokens += 1;
                s.acc.push_str(&token);
                let ev = Event::default().data(json!({ "token": token }).to_string());
                Some((Ok::<_, Infallible>(ev), s))
            }
            Some(StreamItem::Failed) => {
                // Generation was severed part-way. What arrived is NOT the
                // model's answer, so it is deliberately not persisted: stored,
                // it would come back as the assistant's reply on every reload
                // and — with use_history on — be replayed to the model as if
                // it had actually said it, half-sentence and all. Nor is it
                // recorded as a benchmark sample, which would read as a fast
                // short generation rather than a failed one. The cause is in
                // the log (the spawned task above records it); the client is
                // told only that the answer is incomplete.
                tracing::warn!(
                    tokens = s.tokens,
                    "eullm stream: generation interrupted, partial answer discarded"
                );
                let ev = Event::default().data(
                    json!({ "error": "generation interrupted before completion" }).to_string(),
                );
                s.done = true;
                Some((Ok::<_, Infallible>(ev), s))
            }
            None => {
                // Channel closed with no failure marker — a clean end of
                // generation. Persist the assistant reply, if non-empty
                // (see the non-streaming query() for why), and emit the final
                // event.
                let total_generation = s.gen_start.elapsed();
                if !s.acc.trim().is_empty() {
                    let sources_json = crate::rag::sources::history_json(&s.sources);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(text: &str, retrieval_text: Option<&str>) -> ChunkPayload {
        ChunkPayload {
            document_id: "doc".into(),
            chunk_index: 0,
            filename: "f.txt".into(),
            upload_date: String::new(),
            text: text.to_owned(),
            chunk_size: text.len(),
            document_type: "txt".into(),
            structured_fields: None,
            source_start_byte: None,
            source_end_byte: None,
            page_start: None,
            page_end: None,
            provenance_id: None,
            retrieval_text: retrieval_text.map(str::to_owned),
        }
    }

    #[test]
    fn an_empty_question_is_rejected() {
        for q in ["", "   ", "\n\t "] {
            assert!(validate_query(q).is_err(), "q={q:?}");
        }
    }

    #[test]
    fn a_question_at_the_limit_is_accepted() {
        assert!(validate_query(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
    }

    #[test]
    fn an_oversized_question_is_rejected() {
        let err = validate_query(&"a".repeat(MAX_QUERY_CHARS + 1)).unwrap_err();
        assert!(err.contains(&MAX_QUERY_CHARS.to_string()), "err={err:?}");
    }

    /// Characters, not bytes: an accented question of legal length must not
    /// be rejected for being multibyte.
    #[test]
    fn accented_text_counts_as_characters() {
        assert!(validate_query(&"à".repeat(MAX_QUERY_CHARS)).is_ok());
        assert!(validate_query(&"à".repeat(MAX_QUERY_CHARS + 1)).is_err());
    }

    #[test]
    fn context_text_prefers_retrieval_text_when_present() {
        let p = payload("original", Some("[Article 42] original"));
        assert_eq!(context_text(&p), "[Article 42] original");
    }

    #[test]
    fn context_text_falls_back_to_text_when_retrieval_text_absent() {
        let p = payload("original", None);
        assert_eq!(context_text(&p), "original");
    }

    #[test]
    fn absent_top_k_means_the_default() {
        assert_eq!(resolve_top_k(None), retrieval::TOP_K);
    }

    #[test]
    fn present_top_k_is_honored() {
        assert_eq!(resolve_top_k(Some(5)), 5);
        assert_eq!(resolve_top_k(Some(1)), 1);
        assert_eq!(resolve_top_k(Some(MAX_TOP_K)), MAX_TOP_K);
    }

    /// Clamped, not rejected: no client that worked before can break, and
    /// no value can fan out to Qdrant unbounded.
    #[test]
    fn out_of_range_top_k_is_clamped() {
        assert_eq!(resolve_top_k(Some(0)), 1);
        assert_eq!(resolve_top_k(Some(u64::MAX)), MAX_TOP_K);
    }
}
