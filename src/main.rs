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
    tracing::info!(
        build = env!("BUILD_GIT_HASH"),
        host = %settings.server.host,
        port = settings.server.port,
        "starting i3k-rag-engine"
    );

    // Scarica eullm + modelli se assenti, poi avvia eullm in background.
    bootstrap::ensure_ready(&settings.eullm.url).await?;

    let db = db::connect(&settings.database.url).await?;
    db::migrate(&db).await?;
    db::users::seed_admin(&db, settings.auth.admin_default_password.as_deref()).await?;

    // Se il modello non è in cache, scaricalo via reqwest prima di spawn_blocking.
    let model_id = settings.embeddings.model_id.clone();
    ensure_model_downloaded(&model_id).await?;

    // EmbeddingService: caricamento sincrono (bge-m3 ~2.3 GB, GPU/CPU).
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

/// Scarica i file del modello embedding da HuggingFace se non sono in cache locale.
/// Usa reqwest (async, segue redirect CDN automaticamente) invece di hf-hub/ureq
/// che fallisce con RelativeUrlWithoutBase quando il redirect della CDN è gestito
/// manualmente senza join() sull'URL base.
async fn ensure_model_downloaded(model_id: &str) -> anyhow::Result<()> {
    if clients::embeddings::find_model_in_cache(model_id).is_some() {
        return Ok(());
    }

    let dest = clients::embeddings::download_target_dir(model_id);
    tracing::info!(model_id, dest = %dest.display(), "modello non in cache, avvio download da HuggingFace…");
    std::fs::create_dir_all(&dest)
        .with_context(|| format!("crea dir {}", dest.display()))?;

    let client = reqwest::Client::builder()
        .user_agent("i3k-rag-engine/0.1")
        .build()
        .context("reqwest client")?;

    let base = format!("https://huggingface.co/{}/resolve/main", model_id);

    // config.json e tokenizer.json (piccoli, sempre singoli)
    for name in &["config.json", "tokenizer.json"] {
        let url = format!("{base}/{name}");
        let path = dest.join(name);
        if !path.exists() {
            tracing::info!(name, "download…");
            download_file(&client, &url, &path).await
                .with_context(|| format!("download {name}"))?;
        }
    }

    // Pesi: prova prima il file singolo, poi l'indice sharded
    let single = dest.join("model.safetensors");
    if !single.exists() {
        let url = format!("{base}/model.safetensors");
        match download_file(&client, &url, &single).await {
            Ok(()) => tracing::info!("model.safetensors scaricato"),
            Err(_) => {
                // Prova indice sharded
                let idx_name = "model.safetensors.index.json";
                let idx_path = dest.join(idx_name);
                download_file(&client, &format!("{base}/{idx_name}"), &idx_path).await
                    .context("download safetensors index")?;
                let idx: serde_json::Value =
                    serde_json::from_reader(std::fs::File::open(&idx_path)?)
                        .context("parse safetensors index")?;
                let mut shards: Vec<String> = idx["weight_map"]
                    .as_object()
                    .context("weight_map non trovata")?
                    .values()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                shards.sort();
                for shard in shards {
                    let path = dest.join(&shard);
                    if !path.exists() {
                        tracing::info!(shard, "download shard…");
                        download_file(&client, &format!("{base}/{shard}"), &path).await
                            .with_context(|| format!("download shard {shard}"))?;
                    }
                }
            }
        }
    }

    tracing::info!(model_id, "download completato");
    Ok(())
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let resp = client
        .get(url)
        .send()
        .await
        .context("send")?
        .error_for_status()
        .context("HTTP error")?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await.context("crea file")?;
    let mut downloaded: u64 = 0;
    let mut stream = resp;
    let mut last_pct = 0u64;
    while let Some(chunk) = stream.chunk().await.context("chunk")? {
        file.write_all(&chunk).await.context("write")?;
        if total > 0 {
            downloaded += chunk.len() as u64;
            let pct = downloaded * 100 / total;
            if pct / 10 != last_pct / 10 {
                tracing::info!("{:.0}% ({}/{} MB)", pct, downloaded / 1_048_576, total / 1_048_576);
                last_pct = pct;
            }
        }
    }
    file.flush().await.context("flush")?;
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
