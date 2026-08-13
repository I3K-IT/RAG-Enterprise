//! GET /health  GET /info  GET /
//! Public endpoints — no auth required (MAPPA §3).

use axum::{extract::State, Json, response::IntoResponse};
use serde_json::json;

use crate::state::AppState;

/// ingestion_in_progress is true when at least one upload is currently in the
/// heavy phase (extract/chunk/embed). The UI uses it to show an "ingestion in
/// progress" notice to every connected user through the existing polling (see
/// checkBackendHealth in the frontend), not only to whoever started the
/// upload. See state::IngestionGuard.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let active = state.active_ingestions.load(std::sync::atomic::Ordering::SeqCst);
    Json(json!({
        "status": "ok",
        "ingestion_in_progress": active > 0,
    }))
}

/// Exposes the state actually chosen at startup, not merely the desired
/// configuration — embedding_device in particular, so a silent CPU fallback
/// stays visible long after the boot log has scrolled out of the terminal.
/// With swap_during_ingestion=true it reflects the CURRENT state (CPU at rest,
/// GPU during an ingestion), not a fixed boot-time value.
pub async fn info(State(state): State<AppState>) -> impl IntoResponse {
    let (device_label, device_ok) = match state.embeddings.read() {
        Ok(guard) => (
            guard.device_label(),
            guard.device_status() != crate::clients::embeddings::DeviceStatus::CpuFallback,
        ),
        Err(_) => ("cpu (lock poisoned — stato sconosciuto)", false),
    };
    Json(json!({
        "service": "i3k-rag-engine",
        "version": env!("CARGO_PKG_VERSION"),
        "embedding_device": device_label,
        "embedding_device_ok": device_ok,
    }))
}
