//! Axum extractor: reads `Authorization: Bearer <token>`, decodes JWT, returns Claims.
//! Handlers that declare `claims: Claims` get a 401 automatically if the token is absent/invalid.

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    Json,
};
use serde_json::json;

use crate::auth::jwt::{Claims, decode_token};
use crate::state::AppState;

#[async_trait]
impl FromRequestParts<AppState> for Claims {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let token = auth.strip_prefix("Bearer ").ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Authorization header mancante o non Bearer"})),
            )
        })?;

        decode_token(token, &state.settings.auth.jwt_secret).map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "token non valido o scaduto"})),
            )
        })
    }
}
