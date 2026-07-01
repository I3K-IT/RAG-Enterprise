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
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    observability::init_tracing();

    let settings = config::Settings::load()?;
    tracing::info!(
        build = env!("BUILD_GIT_HASH"),
        host = %settings.server.host,
        port = settings.server.port,
        "starting i3k-rag-engine"
    );

    tracing::info!(data_dir = %settings.data.data_path().display(), "data directory");

    // Rende data_dir disponibile al modulo embeddings (find_model_in_cache).
    std::env::set_var("I3K_DATA_DIR", settings.data.data_path());

    // Bootstrap: scarica componenti, avvia qdrant + eullm, attende API ready.
    // guard tiene in vita i processi figlio supervisionati (drop → SIGKILL);
    // espone anche il path GGUF esatto con cui ha avviato eullm (se lo gestisce).
    let guard = bootstrap::ensure_ready(&settings).await?;

    tracing::info!(path = %settings.database.url, "database SQLite");
    let db = db::connect(&settings.database.url).await?;
    db::migrate(&db).await?;
    db::users::seed_admin(&db, settings.auth.admin_default_password.as_deref()).await?;

    let model_id = settings.embeddings.model_id.clone();

    let embeddings = tokio::task::spawn_blocking(move || {
        clients::embeddings::EmbeddingService::load(&model_id)
    })
    .await
    .context("join embedding load")?
    .context("embedding service load")?;
    tracing::info!("embedding service pronto");

    let qdrant = clients::qdrant_store::QdrantStore::new(
        &settings.qdrant.grpc_url,
        &settings.qdrant.collection,
    )
    .await
    .context("qdrant init")?;
    tracing::info!("qdrant pronto");

    // Se bootstrap ha avviato eullm, usa lo STESSO path GGUF come "model" nelle
    // richieste — eullm lo accetta direttamente (vedi ProcessGuard), niente
    // registrazione/import-ollama necessaria. Altrimenti (eullm esterno,
    // manage_subprocesses=false) usa Settings.eullm.model così com'è.
    let eullm_model = guard
        .eullm_model_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| settings.eullm.model.clone());

    let eullm = clients::eullm::EullmClient::new(
        settings.eullm.url.clone(),
        eullm_model,
        settings.eullm.num_ctx,
        settings.eullm.num_predict,
        settings.eullm.repeat_penalty,
        settings.eullm.keep_alive,
    );

    // Warmup: forza il caricamento del modello in VRAM prima di servire traffico
    // reale (parità Python: llm_client.py::warmup(), "avoid timeout on first
    // query"). /api/tags (atteso in bootstrap) conferma solo che il processo
    // eullm è in ascolto, non che il modello sia caricato/pronto per l'inferenza
    // — senza questo passo la prima query reale paga il cold-start e può
    // tornare vuota o incompleta.
    tracing::info!("warmup eullm…");
    match eullm.invoke("hi").await {
        Ok(a) if a.trim().is_empty() => tracing::warn!(
            "eullm warmup: risposta vuota — il modello potrebbe non essere pronto (vedi eullm stesso)"
        ),
        Ok(_) => tracing::info!("eullm warmup completato, modello in VRAM"),
        Err(e) => tracing::warn!(error = %e, "eullm warmup fallito (si caricherà alla prima query reale)"),
    }

    let port = settings.server.port;
    let host = settings.server.host.clone();

    let db_path = settings.database.url.trim_start_matches("sqlite://").to_owned();
    let bk_db = db.clone();
    let bk_qdrant_url = settings.qdrant.url.clone();
    let bk_qdrant_coll = settings.qdrant.collection.clone();
    let bk_dir = settings.backup.dir.clone();
    tokio::spawn(async move {
        if let Err(e) =
            backup::scheduler::start(bk_db, db_path, bk_qdrant_url, bk_qdrant_coll, bk_dir).await
        {
            tracing::warn!(error = %e, "backup scheduler non avviato");
        }
    });

    let app_state =
        state::AppState::new(settings, db, embeddings, Arc::new(qdrant), eullm);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!("server in ascolto su {addr}");

    open_browser(port);

    let router = api::router(app_state);

    // Shutdown graceful: SIGINT (Ctrl+C) o SIGTERM → drop guard → SIGKILL figli.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
        tokio::select! {
            r = axum::serve(listener, router) => r?,
            _ = tokio::signal::ctrl_c() => { tracing::info!("SIGINT ricevuto, shutdown"); }
            _ = sigterm.recv() => { tracing::info!("SIGTERM ricevuto, shutdown"); }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            r = axum::serve(listener, router) => r?,
            _ = tokio::signal::ctrl_c() => { tracing::info!("SIGINT ricevuto, shutdown"); }
        }
    }

    // guard dropped qui → processi figlio SIGKILL'd
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
