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
        .route("/api/auth/users/:id", put(auth::update_user))
        .route("/api/auth/users/:id", delete(auth::delete_user));

    let doc_routes = Router::new()
        .route("/api/documents", get(documents::list))
        .route("/api/documents/upload", post(documents::upload))
        .route("/api/documents/:id", delete(documents::delete))
        .route("/api/documents/:id/download", get(documents::download));

    let admin_routes = Router::new()
        .route("/api/admin/backup", post(admin::trigger_backup))
        .route("/api/admin/backup/list", get(admin::list_backups))
        .route("/api/admin/qdrant/stats", get(admin::qdrant_stats))
        .route("/api/admin/qdrant/documents", get(admin::qdrant_documents))
        .route("/api/admin/qdrant/document/:id", delete(admin::qdrant_delete_document))
        .route("/api/admin/sqlite/documents", get(admin::sqlite_documents));

    let query_routes = Router::new()
        .route("/api/query", post(query::query))
        .route("/api/query/stream", post(query::query_stream))
        .route("/api/chat/history", get(query::chat_history))
        .route("/api/chat/history", delete(query::delete_chat_history));

    let conv_routes = Router::new()
        .route("/api/conversations", get(conversations::list))
        .route("/api/conversations", post(conversations::create))
        .route("/api/conversations/:id", put(conversations::rename))
        .route("/api/conversations/:id", delete(conversations::delete))
        .route("/api/conversations/:id/messages", get(conversations::messages));

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

#[cfg(test)]
mod tests {
    /// axum 0.7 (qui in uso — vedi Cargo.toml) usa la sintassi `:nome` per i path
    /// param. La sintassi `{nome}` è di axum 0.8+: su 0.7 viene trattata come
    /// segmento LETTERALE, quindi non combacia mai con un valore reale. L'unico
    /// servizio che intercetta quelle richieste è il fallback SPA (ServeDir), che
    /// risponde 405 su DELETE/PUT e servirebbe silenziosamente index.html su GET.
    /// Bug reale riscontrato in produzione (delete documenti/qdrant/conversazioni
    /// tutti a 405) — questo test impedisce che si ripresenti.
    #[test]
    fn no_axum_08_style_path_params() {
        let src = include_str!("mod.rs");
        for (i, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(".route(") {
                assert!(
                    !trimmed.contains('{'),
                    "mod.rs:{}: sintassi path param non valida per axum 0.7 (usa :nome, non {{nome}}): {}",
                    i + 1,
                    trimmed
                );
            }
        }
    }
}
