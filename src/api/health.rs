//! GET /health  GET /info  GET /
//! Public endpoints — no auth required (MAPPA §3).

use axum::{extract::State, Json, response::IntoResponse};
use serde_json::json;

use crate::state::AppState;

/// ingestion_in_progress: true se almeno un upload è nella fase pesante
/// (extract/chunk/embed) in questo momento — la UI la usa per mostrare un
/// avviso "ingestione in corso" a tutti gli utenti connessi (polling
/// esistente, vedi frontend checkBackendHealth), non solo a chi ha lanciato
/// l'upload. Vedi state::IngestionGuard.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let active = state.active_ingestions.load(std::sync::atomic::Ordering::SeqCst);
    Json(json!({
        "status": "ok",
        "ingestion_in_progress": active > 0,
    }))
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
