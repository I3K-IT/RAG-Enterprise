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
    let user = match users::find_by_username(&state.db, &body.username).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid credentials"})))
                .into_response();
        }
        Err(e) => {
            tracing::error!("db error in login: {e:#}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "internal error"})))
                .into_response();
        }
    };

    match password::verify(&body.password, &user.password_hash) {
        Ok(true) => {}
        _ => {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid credentials"})))
                .into_response();
        }
    }

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

pub async fn list_users(
    _state: State<AppState>,
    _claims: Claims,
) -> impl IntoResponse {
    Json(json!({"users": [], "total": 0}))
}

pub async fn create_user(
    _state: State<AppState>,
    _claims: Claims,
    Json(_body): Json<UserCreateRequest>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

pub async fn update_user(
    _state: State<AppState>,
    _claims: Claims,
    Path(_user_id): Path<i64>,
) -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

pub async fn delete_user(
    State(_state): State<AppState>,
    claims: Claims,
    Path(user_id): Path<i64>,
) -> impl IntoResponse {
    if user_id == claims.user_id {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "you cannot delete your own account"})),
        )
            .into_response();
    }
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"}))).into_response()
}
