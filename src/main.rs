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

    // EmbeddingService: caricamento sincrono (bge-m3 ~2.3 GB, GPU/CPU).
    let model_id = settings.embeddings.model_id.clone();
    let embeddings = tokio::task::spawn_blocking(move || {
        clients::embeddings::EmbeddingService::load(&model_id)
    })
    .await
    .context("join embedding load")?
    .context("embedding service load")?;
    tracing::info!("embedding service pronto");

    // QdrantStore: connessione + crea collection se assente.
    let qdrant = clients::qdrant_store::QdrantStore::new(
        &settings.qdrant.url,
        &settings.qdrant.collection,
    )
    .await
    .context("qdrant init")?;
    tracing::info!("qdrant pronto");

    let eullm = clients::eullm::EullmClient::new(
        settings.eullm.url.clone(),
        settings.eullm.model.clone(),
        settings.eullm.num_ctx,
        settings.eullm.num_predict,
        settings.eullm.repeat_penalty,
        settings.eullm.keep_alive,
    );

    let port = settings.server.port;
    let host = settings.server.host.clone();

    // Start daily backup scheduler in background (non-blocking — errors logged).
    let db_path = settings.database.url.trim_start_matches("sqlite://").to_owned();
    let bk_db = db.clone();
    let bk_qdrant_url = settings.qdrant.url.clone();
    let bk_qdrant_coll = settings.qdrant.collection.clone();
    let bk_dir = settings.backup.dir.clone();
    tokio::spawn(async move {
        if let Err(e) = backup::scheduler::start(bk_db, db_path, bk_qdrant_url, bk_qdrant_coll, bk_dir).await {
            tracing::warn!(error = %e, "backup scheduler non avviato");
        }
    });

    let app_state = state::AppState::new(settings, db, embeddings, qdrant, eullm);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
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
