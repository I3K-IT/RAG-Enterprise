pub mod auth;
pub mod documents;
pub mod health;
pub mod query;

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use tower_http::cors::CorsLayer;

use crate::state::AppState;

/// Build the full axum Router with all routes and CORS middleware.
pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/health", get(health::health))
        .route("/info", get(health::info));

    let auth_routes = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/change-password", post(auth::change_password))
        .route("/api/auth/users", get(auth::list_users))
        .route("/api/auth/users", post(auth::create_user))
        .route("/api/auth/users/{id}", put(auth::update_user))
        .route("/api/auth/users/{id}", delete(auth::delete_user));

    let doc_routes = Router::new()
        .route("/api/documents", get(documents::list))
        .route("/api/documents/upload", post(documents::upload))
        .route("/api/documents/{id}", delete(documents::delete));

    let query_routes = Router::new()
        .route("/api/query", post(query::query))
        .route("/api/query/stream", post(query::query_stream))
        .route("/api/chat/history", get(query::chat_history))
        .route("/api/chat/history", delete(query::delete_chat_history));

    Router::new()
        .merge(public)
        .merge(auth_routes)
        .merge(doc_routes)
        .merge(query_routes)
        .layer(CorsLayer::permissive())
        .with_state(state)
}
