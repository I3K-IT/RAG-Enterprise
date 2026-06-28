//! Query endpoints (MAPPA §3):
//! POST /api/query         → QueryResponse
//! POST /api/query/stream  → SSE (step / token / done / error)
//! GET  /api/chat/history
//! DELETE /api/chat/history
//! Stubs — Fase 1 implementation.

use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

pub async fn query() -> impl IntoResponse {
    Json(json!({ "error": "not implemented" }))
}

pub async fn query_stream() -> impl IntoResponse {
    Json(json!({ "error": "not implemented" }))
}

pub async fn chat_history() -> impl IntoResponse {
    Json(json!({ "messages": [] }))
}

pub async fn delete_chat_history() -> impl IntoResponse {
    Json(json!({ "deleted": 0 }))
}
