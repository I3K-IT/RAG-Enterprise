//! Endpoints for managing conversations:
//! GET    /api/conversations            → lista conversazioni utente
//! POST   /api/conversations            → crea nuova conversazione
//! PUT    /api/conversations/{id}       → rinomina
//! DELETE /api/conversations/{id}       → elimina + messaggi
//! GET    /api/conversations/{id}/messages → messages of one conversation

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth::jwt::Claims;
use crate::db;
use crate::state::AppState;

fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": msg.to_string() }))).into_response()
}

pub async fn list(State(state): State<AppState>, claims: Claims) -> Response {
    match db::conversations::list_conversations(&state.db, claims.user_id).await {
        Ok(convs) => Json(json!({ "conversations": convs })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub async fn create(State(state): State<AppState>, claims: Claims) -> Response {
    match db::conversations::create_conversation(&state.db, claims.user_id).await {
        Ok(conv) => (StatusCode::CREATED, Json(conv)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub title: String,
}

pub async fn rename(
    State(state): State<AppState>,
    claims: Claims,
    Path(conv_id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Response {
    let title = body.title.trim();
    if title.is_empty() {
        return err(StatusCode::BAD_REQUEST, "title cannot be empty");
    }
    match db::conversations::rename_conversation(
        &state.db,
        &conv_id,
        claims.user_id,
        title,
    )
    .await
    {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "conversation not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    claims: Claims,
    Path(conv_id): Path<String>,
) -> Response {
    match db::conversations::delete_conversation(&state.db, &conv_id, claims.user_id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "conversation not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub async fn messages(
    State(state): State<AppState>,
    claims: Claims,
    Path(conv_id): Path<String>,
) -> Response {
    match db::conversations::list_by_conversation(&state.db, &conv_id, claims.user_id).await {
        Ok(msgs) => Json(json!({ "messages": msgs })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
