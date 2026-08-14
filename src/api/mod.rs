pub mod admin;
pub mod auth;
pub mod conversations;
pub mod documents;
pub mod health;
pub mod query;

use axum::{
    extract::DefaultBodyLimit,
    Router,
    routing::{delete, get, post, put},
};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

/// The directory holding the binary (the portable app dir, see
/// config::default_data_dir) plus "frontend/dist" — not the process CWD.
fn frontend_dist_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("frontend")
        .join("dist")
}

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

    // Upload is isolated in a sub-router: DefaultBodyLimit (2MB) applies to
    // the entire Router it is attached to with .layer(), so it has to be scoped
    // to the upload route alone. The others (list/delete/download, all JSON or
    // bodyless) keep the default. The limit mirrors the Python
    // MAX_UPLOAD_SIZE_MB, default 100MB.
    let max_upload_bytes = (state.settings.storage.max_upload_mb * 1024 * 1024) as usize;
    let upload_route = Router::new()
        .route("/api/documents/upload", post(documents::upload))
        .layer(DefaultBodyLimit::max(max_upload_bytes));

    let doc_routes = Router::new()
        .route("/api/documents", get(documents::list))
        .route("/api/documents/:id", delete(documents::delete))
        .route("/api/documents/:id/download", get(documents::download))
        .merge(upload_route);

    let admin_routes = Router::new()
        .route("/api/admin/backup", post(admin::trigger_backup))
        .route("/api/admin/backup/list", get(admin::list_backups))
        .route("/api/admin/backup/restore", post(admin::restore_backup))
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

    // SPA fallback: serve frontend/dist for all non-API routes. The path is
    // exe-relative, following the same "portable app dir" convention as
    // config::default_data_dir, rather than CWD-relative — otherwise the binary
    // only works when launched from inside the right directory.
    let dist = frontend_dist_dir();
    let spa = ServeDir::new(&dist).not_found_service(ServeFile::new(dist.join("index.html")));

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
    /// axum 0.7, the version in use here (see Cargo.toml), uses `:name` syntax
    /// for path params. The `{name}` syntax belongs to axum 0.8+: on 0.7 it is
    /// treated as a LITERAL segment and so never matches a real value. The only
    /// service that then catches those requests is the SPA fallback (ServeDir),
    /// which
    /// risponde 405 su DELETE/PUT e servirebbe silenziosamente index.html su GET.
    /// Bug reale riscontrato in produzione (delete documenti/qdrant/conversazioni
    /// answered them all with 405 — this test stops that recurring.
    #[test]
    fn no_axum_08_style_path_params() {
        let src = include_str!("mod.rs");
        for (i, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(".route(") {
                assert!(
                    !trimmed.contains('{'),
                    "mod.rs:{}: invalid path-param syntax for axum 0.7 (use :name, not {{name}}): {}",
                    i + 1,
                    trimmed
                );
            }
        }
    }
}
