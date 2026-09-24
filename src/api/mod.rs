pub mod admin;
pub mod auth;
pub mod conversations;
pub mod documents;
pub mod health;
pub mod query;

use axum::{
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, header},
    Router,
    routing::{delete, get, post, put},
};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

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
///
/// `extra_routes`, per extensions::api's doc comment, is the API extension
/// point: a launcher builds its own routes against this same `AppState` and
/// they are merged in here. This crate's own launcher (`lib::run`) always
/// passes `None`.
pub fn router(state: AppState, extra_routes: Option<Router<AppState>>) -> Router {
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
    let max_upload_bytes = state.settings.storage.max_upload_bytes() as usize;
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
    let cors_origins = crate::config::parse_cors_origins(&state.settings.server.cors_origins);

    let dist = frontend_dist_dir();
    let spa = ServeDir::new(&dist).not_found_service(ServeFile::new(dist.join("index.html")));

    let mut app = Router::new()
        .merge(public)
        .merge(auth_routes)
        .merge(doc_routes)
        .merge(query_routes)
        .merge(conv_routes)
        .merge(admin_routes);
    if let Some(extra_routes) = extra_routes {
        app = app.merge(extra_routes);
    }

    // CORS only when someone asked for it. The previous
    // `CorsLayer::permissive()` advertised `Access-Control-Allow-Origin: *`
    // with every method and header, on a binary that serves its own frontend
    // from its own origin — so it protected nothing and permitted everything,
    // letting any page on the internet call this API from a visitor's browser
    // and read the answers. The one legitimate need is a Vite dev server on
    // another port, which is what SERVER__CORS_ORIGINS is for.
    let app = match cors_layer(&cors_origins) {
        Some(layer) => app.layer(layer),
        None => app,
    };

    // Response headers for everything this server returns, API and SPA alike.
    // None of them were set before, on a binary whose whole surface is an
    // administrative UI. Applied here rather than built in a helper because
    // a ServiceBuilder's type spells out every layer it holds, and a type
    // like that breaks the moment a fourth header is added.
    app.layer(SetResponseHeaderLayer::overriding(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    ))
    // Stops a browser second-guessing a declared Content-Type — the classic
    // way an uploaded file ends up executed as something else.
    .layer(SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    ))
    // Document ids and filenames live in these URLs; they have no reason to
    // travel to another site in a Referer header.
    .layer(SetResponseHeaderLayer::overriding(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    ))
    .with_state(state)
    .fallback_service(spa)
}

/// Strict where it counts — `script-src 'self'` means the only runnable code
/// is the bundle this binary ships, and `frame-ancestors 'none'` keeps the
/// admin UI out of anyone else's iframe.
///
/// `'unsafe-inline'` is admitted for styles alone, because the upload
/// progress bar sets its width through a style attribute and CSP blocks
/// those too. A deliberate, narrow concession: style injection is a far
/// smaller problem than script injection, and nothing here relaxes
/// script-src to buy it.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; \
     font-src 'self'; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'self'; \
     form-action 'self'; \
     frame-ancestors 'none'";

/// Builds a CORS layer restricted to `origins`, or `None` when the list is
/// empty — in which case the router attaches no CORS layer at all, and the
/// browser's own same-origin policy is left to do its job undisturbed.
///
/// Deliberately narrow where `permissive()` was not: only the methods this
/// API actually answers, and only the two headers a browser client needs to
/// send. Credentials stay off, because authentication here is a Bearer token
/// the client attaches explicitly, never a cookie the browser would attach
/// on its own.
fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| match o.parse::<HeaderValue>() {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!(origin = %o, "SERVER__CORS_ORIGINS: ignoring unusable origin");
                None
            }
        })
        .collect();
    if parsed.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
    )
}


#[cfg(test)]
mod tests {
    /// axum 0.7, the version in use here (see Cargo.toml), uses `:name` syntax
    /// for path params. The `{name}` syntax belongs to axum 0.8+: on 0.7 it is
    /// treated as a LITERAL segment and so never matches a real value. The only
    /// service that then catches those requests is the SPA fallback (ServeDir),
    /// which answers 405 to DELETE/PUT and would silently serve index.html on
    /// GET. A real bug, seen in production: deleting documents, Qdrant data and
    /// conversations all answered 405 — this test stops that recurring.
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
