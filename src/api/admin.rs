//! Admin endpoints (admin role required):
//! POST   /api/admin/backup                  → trigger immediate backup
//! GET    /api/admin/backup/list             → list available archives
//! GET    /api/admin/qdrant/stats            → collection stats
//! GET    /api/admin/qdrant/documents        → unique documents in Qdrant
//! DELETE /api/admin/qdrant/document/{id}   → delete all vectors for a document
//! GET    /api/admin/sqlite/documents        → all rows (inclusi soft-deleted)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::auth::{jwt::Claims, rbac::Role};
use crate::backup::service;
use crate::db;
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

// ── GET /api/admin/qdrant/stats ───────────────────────────────────────────────

pub async fn qdrant_stats(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    let url = format!("{}/collections/{}", state.settings.qdrant.url, state.settings.qdrant.collection);
    match reqwest::get(&url).await {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(body) => Json(body).into_response(),
            Err(e) => err(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

// ── GET /api/admin/qdrant/documents ──────────────────────────────────────────

pub async fn qdrant_documents(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    let url = format!(
        "{}/collections/{}/points/scroll",
        state.settings.qdrant.url, state.settings.qdrant.collection
    );
    let body = json!({ "limit": 10000, "with_payload": true, "with_vector": false });
    let client = reqwest::Client::new();
    let resp = match client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, e),
    };
    let raw: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_GATEWAY, e),
    };

    // Raggruppa per document_id dai payload
    let mut docs: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    if let Some(points) = raw.get("result").and_then(|r| r.get("points")).and_then(|p| p.as_array()) {
        for point in points {
            if let Some(payload) = point.get("payload") {
                let doc_id = payload.get("document_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if doc_id.is_empty() { continue; }
                let entry = docs.entry(doc_id.clone()).or_insert_with(|| json!({
                    "document_id": doc_id,
                    "filename": payload.get("filename").and_then(|v| v.as_str()).unwrap_or(""),
                    "upload_date": payload.get("upload_date").and_then(|v| v.as_str()).unwrap_or(""),
                    "chunk_count": 0_i64,
                }));
                if let Some(n) = entry.get_mut("chunk_count").and_then(|v| v.as_i64()) {
                    *entry.get_mut("chunk_count").unwrap() = json!(n + 1);
                }
            }
        }
    }
    let mut list: Vec<serde_json::Value> = docs.into_values().collect();
    list.sort_by(|a, b| {
        let da = a.get("upload_date").and_then(|v| v.as_str()).unwrap_or("");
        let db = b.get("upload_date").and_then(|v| v.as_str()).unwrap_or("");
        db.cmp(da)
    });
    Json(json!({ "documents": list })).into_response()
}

// ── DELETE /api/admin/qdrant/document/{id} ────────────────────────────────────

pub async fn qdrant_delete_document(
    State(state): State<AppState>,
    claims: Claims,
    Path(document_id): Path<String>,
) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    let url = format!(
        "{}/collections/{}/points/delete",
        state.settings.qdrant.url, state.settings.qdrant.collection
    );
    let body = json!({
        "filter": {
            "must": [{ "key": "document_id", "match": { "value": document_id } }]
        }
    });
    let client = reqwest::Client::new();
    match client.post(&url).json(&body).send().await {
        Ok(r) if r.status().is_success() => Json(json!({ "deleted": true })).into_response(),
        Ok(r) => {
            let status = r.status().as_u16();
            let msg = r.text().await.unwrap_or_default();
            err(StatusCode::BAD_GATEWAY, format!("qdrant {status}: {msg}"))
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

// ── GET /api/admin/sqlite/documents ──────────────────────────────────────────

pub async fn sqlite_documents(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    match db::documents::list_all(&state.db).await {
        Ok(docs) => Json(json!({ "documents": docs })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
