//! GET /health  GET /info  GET /
//! Public endpoints — no auth required (MAPPA §3).

use axum::{extract::State, Json, response::IntoResponse};
use serde_json::json;

use crate::state::AppState;

pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// Espone lo stato reale scelto all'avvio (non solo la config desiderata) —
/// in particolare embedding_device, cosi' un fallback CPU silenzioso (5a) resta
/// visibile anche dopo che il log di boot e' scomparso dalla history del terminale.
pub async fn info(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "service": "i3k-rag-engine",
        "version": env!("CARGO_PKG_VERSION"),
        "embedding_device": state.embeddings.device_label(),
        "embedding_device_ok": state.embeddings.device_status() != crate::clients::embeddings::DeviceStatus::CpuFallback,
    }))
}
