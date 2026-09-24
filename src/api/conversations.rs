//! Endpoints for managing conversations:
//! GET    /api/conversations            → the user's conversations
//! POST   /api/conversations            → create a new conversation
//! PUT    /api/conversations/{id}       → rename
//! DELETE /api/conversations/{id}       → delete, with its messages
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

/// A 4xx tells the caller what they got wrong, so its message travels.
/// A 5xx does not: `msg` is an anyhow chain carrying whatever context the
/// failure picked up on the way out — filesystem paths, the Qdrant URL, SQL
/// text, the body of an eullm reply — and handing that to an unauthenticated
/// caller is free reconnaissance. The detail goes to the log, where it is
/// actually useful, and the response says only that something broke.
fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    if status.is_server_error() {
        tracing::error!(status = %status, detail = %msg, "request failed");
        return (status, Json(json!({ "error": "internal server error" }))).into_response();
    }
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

/// Longest conversation title this API will store.
///
/// The only limit before was axum's 2 MB default body: a megabyte-scale
/// title was stored verbatim and returned on every list call, while the
/// frontend only ever needs ~50 characters for the sidebar. Two hundred
/// characters is far past any real title, and the rejection is cheap
/// because it happens before the database.
pub const MAX_TITLE_CHARS: usize = 200;

/// `Err(message)` when the title is empty or too long. A free function so
/// the rule can be tested without a running server, like `validate_query`
/// and `validate_new_password` elsewhere in this codebase.
pub(crate) fn validate_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("title cannot be empty".to_owned());
    }
    // Characters, not bytes: an accented title must not count double
    // against a limit expressed to the user in characters.
    let len = title.chars().count();
    if len > MAX_TITLE_CHARS {
        return Err(format!(
            "the title is {len} characters long; the maximum is {MAX_TITLE_CHARS}"
        ));
    }
    Ok(())
}

pub async fn rename(
    State(state): State<AppState>,
    claims: Claims,
    Path(conv_id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Response {
    if let Err(msg) = validate_title(&body.title) {
        return err(StatusCode::BAD_REQUEST, msg);
    }
    let title = body.title.trim();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_blank_titles_fail() {
        for title in ["", "   ", "\n\t "] {
            assert!(validate_title(title).is_err(), "title={title:?}");
        }
    }

    #[test]
    fn boundary_lengths_pass() {
        assert!(validate_title("Weekly sync").is_ok());
        assert!(validate_title(&"x".repeat(MAX_TITLE_CHARS)).is_ok());
    }

    #[test]
    fn overlong_title_fails() {
        let err = validate_title(&"x".repeat(MAX_TITLE_CHARS + 1)).unwrap_err();
        assert!(err.contains(&MAX_TITLE_CHARS.to_string()), "err={err}");
    }

    #[test]
    fn multibyte_chars_count_once() {
        assert!(validate_title(&"à".repeat(MAX_TITLE_CHARS)).is_ok());
        assert!(validate_title(&"à".repeat(MAX_TITLE_CHARS + 1)).is_err());
    }
}
