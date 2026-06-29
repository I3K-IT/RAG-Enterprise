//! GET /health  GET /info  GET /
//! Public endpoints — no auth required (MAPPA §3).

use axum::{Json, response::IntoResponse};
use serde_json::json;

pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

pub async fn info() -> impl IntoResponse {
    Json(json!({
        "service": "i3k-rag-engine",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
