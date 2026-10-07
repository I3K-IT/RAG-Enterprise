//! POST /api/auth/login
//! GET  /api/auth/me
//! POST /api/auth/change-password
//! Admin stubs: GET/POST /api/auth/users, PUT/DELETE /api/auth/users/{id}

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::{
    jwt::{Claims, create_token},
    password,
};
use crate::db::users;
use crate::state::AppState;

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub role: String,
    pub created_at: String,
    pub last_login: Option<String>,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub token_type: String,
    pub user: UserInfo,
}

#[derive(Deserialize)]
pub struct PasswordChangeRequest {
    pub old_password: String,
    pub new_password: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct UserCreateRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub role: Option<String>,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    // Progressive delay for an account that keeps failing — see auth::throttle
    // for the whole design, including why none of this looks at the client's
    // IP. Applied before the lookup and keyed on the name exactly as
    // submitted, so that spraying usernames is counted too and the throttle
    // itself never reveals which accounts exist.
    let delay = state.login_throttle.delay_for(&body.username);
    if !delay.is_zero() {
        tracing::warn!(
            username = %body.username,
            delay_ms = delay.as_millis() as u64,
            "login throttled after repeated failures"
        );
        tokio::time::sleep(delay).await;
    }

    // Bounded concurrency around the Argon2 verification below. Refusing here
    // costs nothing, which is the point: without it every attempt buys a hash,
    // and the cost of rejecting an attack would scale with the attack. The
    // permit is released on Drop, so every early return below frees its slot.
    let _verification_slot = match state.login_throttle.try_begin_verification() {
        Some(permit) => permit,
        None => {
            tracing::warn!(
                username = %body.username,
                "login refused: all password-verification slots busy"
            );
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "too many login attempts in flight, try again shortly"})),
            )
                .into_response();
        }
    };

    let user = match users::find_by_username(&state.db, &body.username).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            state.login_throttle.record_failure(&body.username);
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid credentials"})))
                .into_response();
        }
        Err(e) => {
            // Our fault, not the caller's: do not count it against them.
            tracing::error!("db error in login: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "internal error"})))
                .into_response();
        }
    };

    match password::verify(&body.password, &user.password_hash) {
        Ok(true) => {}
        _ => {
            state.login_throttle.record_failure(&body.username);
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid credentials"})))
                .into_response();
        }
    }

    state.login_throttle.record_success(&body.username);

    if let Err(e) = users::touch_last_login(&state.db, user.id).await {
        tracing::warn!("touch_last_login failed: {e:#}");
    }

    let token = match create_token(
        user.id,
        &user.username,
        user.role(),
        &state.settings.auth.jwt_secret,
        state.settings.auth.jwt_expiry_minutes,
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("JWT create failed: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "internal error"})))
                .into_response();
        }
    };

    tracing::info!(username = %user.username, role = %user.role, "login ok");
    Json(LoginResponse {
        access_token: token,
        token_type: "bearer".into(),
        user: UserInfo {
            id: user.id,
            username: user.username,
            email: user.email,
            role: user.role,
            created_at: user.created_at,
            last_login: user.last_login,
        },
    })
    .into_response()
}

pub async fn me(
    State(state): State<AppState>,
    claims: Claims,
) -> impl IntoResponse {
    match users::find_by_id(&state.db, claims.user_id).await {
        Ok(Some(u)) => Json(UserInfo {
            id: u.id,
            username: u.username,
            email: u.email,
            role: u.role,
            created_at: u.created_at,
            last_login: u.last_login,
        })
        .into_response(),
        Ok(None) => {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "user not found"}))).into_response()
        }
        Err(e) => {
            tracing::error!("db error in me: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "internal error"})))
                .into_response()
        }
    }
}

pub async fn change_password(
    State(state): State<AppState>,
    claims: Claims,
    Json(body): Json<PasswordChangeRequest>,
) -> impl IntoResponse {
    let user = match users::find_by_id(&state.db, claims.user_id).await {
        Ok(Some(u)) => u,
        _ => {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "user not found"})))
                .into_response();
        }
    };

    match password::verify(&body.old_password, &user.password_hash) {
        Ok(true) => {}
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "current password is incorrect"})),
            )
                .into_response();
        }
    }

    // Enforce server-side the floor the UI advertises (plus a ceiling it
    // never had): without this, a direct API call could set an empty or
    // gigabyte-long password regardless of what the form allows.
    if let Err(e) = password::validate_new_password(&body.new_password) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response();
    }

    let new_hash = match password::hash(&body.new_password) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("password hash error: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "internal error"})))
                .into_response();
        }
    };

    if let Err(e) = users::update_password(&state.db, claims.user_id, &new_hash).await {
        tracing::error!("update_password error: {e:#}");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "internal error"})))
            .into_response();
    }

    Json(json!({"message": "password changed"})).into_response()
}

// ── Admin-only stubs ──────────────────────────────────────────────────────────

// Fail closed while the real user management is unimplemented: without this,
// any authenticated role (including plain `user`) gets a 200 from list_users
// today, and whatever gets built on these stubs later inherits the same
// hole. Mirrors api::admin::require_admin's "admin role required" contract.
fn require_admin(claims: &Claims) -> Option<(StatusCode, Json<serde_json::Value>)> {
    if !claims.role.can_manage_users() {
        Some((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "admin role required"})),
        ))
    } else {
        None
    }
}

/// One row of the `list_users` answer: everything the admin panel shows,
/// and deliberately not the password hash. Serialising `UserRow` straight
/// through would hand every account's Argon2 hash to whoever calls this
/// endpoint, which is exactly what this separate shape exists to prevent.
#[derive(serde::Serialize)]
struct PublicUser {
    id: i64,
    username: String,
    email: String,
    role: String,
}

pub async fn list_users(State(state): State<AppState>, claims: Claims) -> impl IntoResponse {
    if let Some(r) = require_admin(&claims) {
        return r.into_response();
    }
    match users::list_all(&state.db).await {
        Ok(rows) => {
            let users: Vec<PublicUser> = rows
                .into_iter()
                .map(|u| PublicUser {
                    id: u.id,
                    username: u.username,
                    email: u.email,
                    role: u.role,
                })
                .collect();
            let total = users.len();
            Json(json!({ "users": users, "total": total })).into_response()
        }
        Err(e) => {
            tracing::error!("list_users error: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal error"})),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of `PublicUser` existing as a separate shape: there is
    /// deliberately no field for a hash to travel in, so serialising a row for
    /// the admin panel cannot leak one even by accident.
    #[test]
    fn the_public_user_shape_cannot_carry_a_password_hash() {
        let public = PublicUser {
            id: 1,
            username: "alice".to_owned(),
            email: "alice@example.com".to_owned(),
            role: "admin".to_owned(),
        };
        let body = serde_json::to_string(&public).expect("serialize");
        assert!(
            !body.to_ascii_lowercase().contains("password"),
            "the shape must not name it: {body}"
        );
        assert!(
            !body.to_ascii_lowercase().contains("hash"),
            "nor carry it under another name: {body}"
        );
        // And it is exactly what the admin panel reads.
        let value: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(value["id"], 1);
        assert_eq!(value["username"], "alice");
        assert_eq!(value["email"], "alice@example.com");
        assert_eq!(value["role"], "admin");
    }
}

pub async fn create_user(
    _state: State<AppState>,
    claims: Claims,
    Json(_body): Json<UserCreateRequest>,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&claims) {
        return r.into_response();
    }
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not implemented"})),
    )
        .into_response()
}

pub async fn update_user(
    _state: State<AppState>,
    claims: Claims,
    Path(_user_id): Path<i64>,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&claims) {
        return r.into_response();
    }
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not implemented"})),
    )
        .into_response()
}

pub async fn delete_user(
    State(_state): State<AppState>,
    claims: Claims,
    Path(user_id): Path<i64>,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&claims) {
        return r.into_response();
    }
    if user_id == claims.user_id {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "you cannot delete your own account"})),
        )
            .into_response();
    }
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not implemented"})),
    )
        .into_response()
}
