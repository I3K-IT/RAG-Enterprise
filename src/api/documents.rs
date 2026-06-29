//! Document management endpoints (MAPPA §3):
//! GET    /api/documents
//! GET    /api/documents/{id}/download
//! POST   /api/documents/upload          (require_upload_permission)
//! POST   /api/documents/upload-batch    (require_upload_permission)
//! DELETE /api/documents/{id}            (require_delete_permission)
//! Stubs — Fase 1 implementation.

use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

pub async fn list() -> impl IntoResponse {
    Json(json!({ "documents": [] }))
}

pub async fn upload() -> impl IntoResponse {
    Json(json!({ "error": "not implemented" }))
}

pub async fn delete() -> impl IntoResponse {
    Json(json!({ "error": "not implemented" }))
}
