mod api;
mod auth;
mod backup;
mod bootstrap;
mod clients;
mod config;
mod db;
mod documents;
mod error;
mod license;
mod observability;
mod rag;
mod state;

use anyhow::{Context, Result};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    observability::init_tracing();

    let settings = config::Settings::load()?;
    tracing::info!(host = %settings.server.host, port = settings.server.port, "starting i3k-rag-engine");

    // Scarica eullm + modelli se assenti, poi avvia eullm in background.
    bootstrap::ensure_ready(&settings.eullm.url).await?;

    let db = db::connect(&settings.database.url).await?;
    db::migrate(&db).await?;
    db::users::seed_admin(&db, settings.auth.admin_default_password.as_deref()).await?;

    let port = settings.server.port;
    let host = settings.server.host.clone();
    let app_state = state::AppState::new(settings, db);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!("server in ascolto su {addr}");

    // Apre il browser dopo che il server è in ascolto (500 ms di margine).
    open_browser(port);

    let router = api::router(app_state);
    axum::serve(listener, router).await?;
    Ok(())
}

fn open_browser(port: u16) {
    let url = format!("http://localhost:{port}");
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        tracing::info!("apertura browser: {url}");
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    });
}
