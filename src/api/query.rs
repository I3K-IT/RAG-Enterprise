//! Query endpoints (MAPPA §5):
//! POST /api/query         → { answer, sources }
//! POST /api/query/stream  → SSE  { token } … { done, sources }
//! GET  /api/chat/history  → { messages }
//! DELETE /api/chat/history → { deleted }

use std::convert::Infallible;

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
    /// ID della conversazione SQLite. Se presente, i messaggi vengono salvati
    /// in quella conversazione e la history viene letta solo da essa.
    pub conversation_id: Option<String>,
}

fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": msg.to_string() }))).into_response()
}

/// eullm è scaricato dalla VRAM (o in procinto) per via di
/// `unload_during_ingestion` — vedi AppState::ingestion_blocks_queries.
fn ingestion_busy_response() -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "ingestione documento in corso: il modello è temporaneamente scaricato dalla VRAM, riprova tra qualche secondo",
    )
}

// ── Shared setup: embed → search → load history → build prompt ────────────────

async fn prepare(
    state: &AppState,
    question: &str,
    user_id: i64,
    use_history: bool,
    conversation_id: Option<&str>,
) -> anyhow::Result<(String, Vec<Source>)> {
    // 1. Embed query (CPU/GPU bound)
    let svc = state.embeddings.clone();
    let q = question.to_owned();
    let query_vec = tokio::task::spawn_blocking(move || svc.embed_text(&q)).await??;

    // 2. Vector search (top_k=15, threshold=0.30 — MAPPA §5)
    let hits = state
        .qdrant
        .search(query_vec, retrieval::TOP_K, Some(retrieval::RELEVANCE_THRESHOLD))
        .await?;

    // 3. Build sources and context string
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

    // 4. Load last 3 exchanges from history if requested
    let history = if use_history {
        build_history_pairs(state, user_id, conversation_id).await?
    } else {
        vec![]
    };

    let full_prompt = prompt::build_prompt(&context, question, &history);
    tracing::info!(
        chars = full_prompt.len(),
        chunks = sources.len(),
        history_pairs = history.len(),
        "prompt costruito"
    );
    Ok((full_prompt, sources))
}

async fn build_history_pairs(
    state: &AppState,
    user_id: i64,
    conversation_id: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let msgs = if let Some(cid) = conversation_id {
        // Prendi gli ultimi 6 messaggi di questa conversazione
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
    let (full_prompt, sources) =
        match prepare(&state, &req.query, claims.user_id, req.use_history, conv_id).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };

    let answer = match state.eullm.invoke(&full_prompt).await {
        Ok(a) => a,
        Err(e) => return err(StatusCode::BAD_GATEWAY, format!("eullm: {e}")),
    };

    // Risposta vuota (es. il modello ha prodotto solo un blocco <think> poi
    // ripulito): NON persistere — un turno "assistant" vuoto in cronologia
    // contaminerebbe i prompt dei tentativi successivi (use_history=true).
    if answer.trim().is_empty() {
        tracing::warn!(query = %req.query, "eullm: risposta vuota");
        return err(StatusCode::BAD_GATEWAY, "il modello non ha prodotto una risposta, riprova");
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
    let (full_prompt, sources) =
        match prepare(&state, &req.query, claims.user_id, req.use_history, conv_id).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };

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

    // Convert mpsc receiver into an SSE stream.
    // State: (rx, accumulated_answer, sources, db_pool, user_id, conv_id, is_done)
    let db = state.db.clone();
    let uid = claims.user_id;
    let stream_conv_id = req.conversation_id.clone();
    let stream = unfold(
        (rx, String::new(), sources, db, uid, stream_conv_id, false),
        |(mut rx, mut acc, sources, db, uid, cid, done)| async move {
            if done {
                return None;
            }
            match rx.recv().await {
                Some(token) => {
                    acc.push_str(&token);
                    let ev = Event::default().data(json!({ "token": token }).to_string());
                    Some((Ok::<_, Infallible>(ev), (rx, acc, sources, db, uid, cid, false)))
                }
                None => {
                    // Channel closed — persist assistant reply (se non vuota: vedi
                    // query() non-streaming per il motivo) ed emette l'evento finale.
                    if !acc.trim().is_empty() {
                        let sources_json = serde_json::to_string(&sources).unwrap_or_default();
                        let _ = db::conversations::insert(
                            &db, uid, "assistant", &acc, Some(&sources_json),
                            cid.as_deref(),
                        )
                        .await;
                    } else {
                        tracing::warn!("eullm stream: risposta vuota, non persistita");
                    }
                    let ev = Event::default()
                        .data(json!({ "done": true, "sources": sources }).to_string());
                    Some((Ok::<_, Infallible>(ev), (rx, acc, vec![], db, uid, cid, true)))
                }
            }
        },
    );

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
