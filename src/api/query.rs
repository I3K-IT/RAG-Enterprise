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

#[derive(Deserialize)]
pub struct QueryRequest {
    pub question: String,
    #[serde(default)]
    pub use_history: bool,
}

fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": msg.to_string() }))).into_response()
}

// ── Shared setup: embed → search → load history → build prompt ────────────────

async fn prepare(
    state: &AppState,
    question: &str,
    user_id: i64,
    use_history: bool,
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
        build_history_pairs(state, user_id).await?
    } else {
        vec![]
    };

    let full_prompt = prompt::build_prompt(&context, question, &history);
    Ok((full_prompt, sources))
}

async fn build_history_pairs(
    state: &AppState,
    user_id: i64,
) -> anyhow::Result<Vec<(String, String)>> {
    // List returns DESC; we reverse to get chronological order.
    let msgs = db::conversations::list_by_user(&state.db, user_id, 6).await?;
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
    let (full_prompt, sources) =
        match prepare(&state, &req.question, claims.user_id, req.use_history).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };

    let answer = match state.eullm.invoke(&full_prompt).await {
        Ok(a) => a,
        Err(e) => return err(StatusCode::BAD_GATEWAY, format!("eullm: {e}")),
    };

    // Persist conversation
    let sources_json = serde_json::to_string(&sources).unwrap_or_default();
    let _ = db::conversations::insert(&state.db, claims.user_id, "user", &req.question, None).await;
    let _ = db::conversations::insert(
        &state.db,
        claims.user_id,
        "assistant",
        &answer,
        Some(&sources_json),
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
    // Run setup synchronously before opening the SSE stream so we can return
    // a proper HTTP error if embed/search fails.
    let (full_prompt, sources) =
        match prepare(&state, &req.question, claims.user_id, req.use_history).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };

    // Persist user question (answer is stored when the stream finishes).
    let _ =
        db::conversations::insert(&state.db, claims.user_id, "user", &req.question, None).await;

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
    // State: (rx, accumulated_answer, sources, db_pool, user_id, is_done)
    let db = state.db.clone();
    let uid = claims.user_id;
    let stream = unfold(
        (rx, String::new(), sources, db, uid, false),
        |(mut rx, mut acc, sources, db, uid, done)| async move {
            if done {
                return None;
            }
            match rx.recv().await {
                Some(token) => {
                    acc.push_str(&token);
                    let ev = Event::default().data(json!({ "token": token }).to_string());
                    Some((Ok::<_, Infallible>(ev), (rx, acc, sources, db, uid, false)))
                }
                None => {
                    // Channel closed — persist assistant reply and emit final event.
                    let sources_json = serde_json::to_string(&sources).unwrap_or_default();
                    let _ = db::conversations::insert(
                        &db, uid, "assistant", &acc, Some(&sources_json),
                    )
                    .await;
                    let ev = Event::default()
                        .data(json!({ "done": true, "sources": sources }).to_string());
                    Some((Ok::<_, Infallible>(ev), (rx, acc, vec![], db, uid, true)))
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
