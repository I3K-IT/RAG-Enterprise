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

#[tokio::main]
async fn main() -> Result<()> {
    observability::init_tracing();

    let settings = config::Settings::load()?;
    tracing::info!(host = %settings.server.host, port = settings.server.port, "starting i3k-rag-engine");

    // Scarica eullm + modelli se assenti, poi avvia eullm in background.
    bootstrap::ensure_ready(&settings.eullm.url).await?;

    let db = db::connect(&settings.database.url).await?;
    db::migrate(&db).await?;

    let _state = state::AppState::new(settings, db);

    // TODO Fase 1: build router + bind axum server
    todo!("axum router + server bind — Fase 1")
}
