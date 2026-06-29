//! Admin endpoints (admin role required):
//! POST /api/admin/backup         → trigger immediate backup
//! GET  /api/admin/backup/list    → list available archives

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::auth::{jwt::Claims, rbac::Role};
use crate::backup::service;
use crate::state::AppState;

fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": msg.to_string() }))).into_response()
}

fn require_admin(claims: &Claims) -> Option<Response> {
    if claims.role != Role::Admin {
        Some(err(StatusCode::FORBIDDEN, "admin role required"))
    } else {
        None
    }
}

// ── POST /api/admin/backup ────────────────────────────────────────────────────

pub async fn trigger_backup(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) {
        return r;
    }

    // Extract the filesystem path from the sqlite:// URL
    let db_path = state
        .settings
        .database
        .url
        .trim_start_matches("sqlite://")
        .to_owned();

    match service::create_backup(
        &state.db,
        &db_path,
        &state.settings.qdrant.url,
        &state.settings.qdrant.collection,
        &state.settings.backup.dir,
    )
    .await
    {
        Ok(path) => Json(json!({
            "ok": true,
            "archive": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ── GET /api/admin/backup/list ────────────────────────────────────────────────

pub async fn list_backups(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) {
        return r;
    }
    let archives = service::list_backups(&state.settings.backup.dir);
    Json(json!({ "backups": archives })).into_response()
}
