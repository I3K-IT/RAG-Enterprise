pub mod admin;
pub mod auth;
pub mod conversations;
pub mod documents;
pub mod health;
pub mod query;

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

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
        .route("/api/documents/{id}", delete(documents::delete))
        .route("/api/documents/{id}/download", get(documents::download));

    let admin_routes = Router::new()
        .route("/api/admin/backup", post(admin::trigger_backup))
        .route("/api/admin/backup/list", get(admin::list_backups))
        .route("/api/admin/qdrant/stats", get(admin::qdrant_stats))
        .route("/api/admin/qdrant/documents", get(admin::qdrant_documents))
        .route("/api/admin/qdrant/document/{id}", delete(admin::qdrant_delete_document))
        .route("/api/admin/sqlite/documents", get(admin::sqlite_documents));

    let query_routes = Router::new()
        .route("/api/query", post(query::query))
        .route("/api/query/stream", post(query::query_stream))
        .route("/api/chat/history", get(query::chat_history))
        .route("/api/chat/history", delete(query::delete_chat_history));

    let conv_routes = Router::new()
        .route("/api/conversations", get(conversations::list))
        .route("/api/conversations", post(conversations::create))
        .route("/api/conversations/{id}", put(conversations::rename))
        .route("/api/conversations/{id}", delete(conversations::delete))
        .route("/api/conversations/{id}/messages", get(conversations::messages));

    // SPA fallback: serve frontend/dist for all non-API routes
    let spa = ServeDir::new("frontend/dist")
        .not_found_service(ServeFile::new("frontend/dist/index.html"));

    Router::new()
        .merge(public)
        .merge(auth_routes)
        .merge(doc_routes)
        .merge(query_routes)
        .merge(conv_routes)
        .merge(admin_routes)
        .layer(CorsLayer::permissive())
        .with_state(state)
        .fallback_service(spa)
}
