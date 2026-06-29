//! POST /api/auth/login
//! GET  /api/auth/me
//! POST /api/auth/change-password
//! Admin: GET/POST /api/auth/users, PUT/DELETE /api/auth/users/{id}
//! Stubs — Fase 1 implementation.

use axum::response::IntoResponse;
use serde_json::json;
use axum::Json;

pub async fn login() -> impl IntoResponse {
    // TODO Fase 1
    Json(json!({ "error": "not implemented" }))
}

pub async fn me() -> impl IntoResponse {
    Json(json!({ "error": "not implemented" }))
}

pub async fn change_password() -> impl IntoResponse {
    Json(json!({ "error": "not implemented" }))
}
