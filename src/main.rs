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

use anyhow::Result;
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

    let port = settings.server.port;
    let _state = state::AppState::new(settings, db);

    // Apre il browser dopo che il server è in ascolto (500 ms di margine).
    // Il tokio::spawn viene schedulato ora; la chiamata xdg-open parte dopo il bind axum.
    open_browser(port);

    // TODO Fase 1: build router + bind axum server
    todo!("axum router + server bind — Fase 1")
}

fn open_browser(port: u16) {
    let url = format!("http://localhost:{port}");
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        tracing::info!("apertura browser: {url}");
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    });
}
