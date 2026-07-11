mod api;
mod auth;
mod backup;
mod bench;
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

/// -V/--version: stampa e esce SUBITO, prima di qualunque bootstrap (tracing,
/// config, qdrant/eullm) — vedi CARGO_PKG_VERSION note sopra sul disallineamento
/// tag/Cargo.toml: include anche BUILD_GIT_HASH (build.rs) perché la versione
/// da sola non basta a distinguere due build con lo stesso Cargo.toml.
fn is_version_flag(args: &[String]) -> bool {
    args.iter().any(|a| a == "-V" || a == "--version")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if is_version_flag(&args) {
        println!("i3k-rag-engine {} ({})", env!("CARGO_PKG_VERSION"), env!("BUILD_GIT_HASH"));
        return Ok(());
    }

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

    // Bootstrap: scarica componenti, avvia qdrant (+ eullm, ordine sotto),
    // attende API ready. guard tiene in vita i processi figlio supervisionati
    // (drop → SIGKILL); espone anche il path GGUF esatto con cui ha avviato
    // eullm (se lo gestisce).
    //
    // Ordine di avvio eullm/embedding: dipende da settings.eullm.fit.
    //   fit=false (default, es. x86_64 pinnato su eullm senza --fit oggi):
    //     eullm PRIMA, poi embedding — eullm non si adatta alla VRAM già
    //     occupata, quindi deve prendersi la sua allocazione fissa per primo
    //     (vedi audit Fase 1, punto 5a: il warmup forza il caricamento reale,
    //     non solo /api/tags che conferma solo il processo in ascolto).
    //   fit=true (eullm ≥ v0.6.9, --fit disponibile — vedi EullmSettings::fit):
    //     embedding PRIMA, poi eullm con --fit — --fit legge la VRAM libera
    //     con cudaMemGetInfo al proprio avvio, quindi deve vedere la VRAM già
    //     ridotta dall'embedding per offloadare i layer di conseguenza
    //     (altrimenti i due si contenderebbero la VRAM al boot).
    // _guard: usato solo per Drop (kill_on_drop dei processi figlio) da qui
    // in poi — il path GGUF di eullm è già stato consumato da
    // build_eullm_client() dentro ciascun branch, prima di questa tupla.
    let (_guard, mut embeddings, eullm) = if settings.eullm.fit {
        tracing::info!(
            "eullm.fit=true: carico l'embedding PRIMA di avviare eullm, così --fit vede la VRAM già ridotta"
        );
        let (phase1, qdrant_children) = bootstrap::provision_and_start_qdrant(&settings).await?;
        let embeddings = load_embedding(&settings).await?;
        tracing::info!(device = embeddings.device_label(), "embedding service pronto (prima di eullm)");
        let guard = bootstrap::start_eullm(&settings, phase1, qdrant_children).await?;
        let eullm = build_eullm_client(&settings, &guard).await;
        warmup_eullm(&eullm).await;
        (guard, embeddings, eullm)
    } else {
        let guard = bootstrap::ensure_ready(&settings).await?;
        let eullm = build_eullm_client(&settings, &guard).await;
        warmup_eullm(&eullm).await;
        let embeddings = load_embedding(&settings).await?;
        tracing::info!(device = embeddings.device_label(), "embedding service pronto");
        (guard, embeddings, eullm)
    };

    // --bench/--benchmark: riusa lo stesso bootstrap (qdrant+eullm+embedding)
    // di sopra, poi esce subito — niente DB, niente server HTTP. _guard resta
    // in scope fino al return di main(), quindi qdrant/eullm vengono comunque
    // spenti correttamente a benchmark concluso (Drop → SIGKILL figli).
    if let Some(bench_args) = bench::parse_args(&args) {
        tracing::info!(doc = %bench_args.doc_path.display(), "modalità benchmark");
        return bench::run(&settings, &bench_args, &mut embeddings, Arc::new(eullm)).await;
    }

    // --bench-live: server e frontend partono normalmente, ma ogni
    // ingestione/query reale viene cronometrata (vedi api/documents.rs,
    // api/query.rs) e accumulata qui. Report scritto alla chiusura, sotto.
    let live_bench = if bench::live_mode_requested(&args) {
        tracing::info!(
            "--bench-live attivo: registro tempi/hardware di ogni ingestione e query reali di questa sessione, report scritto alla chiusura"
        );
        Some(Arc::new(bench::LiveRecorder::new(&embeddings, &settings.eullm.model)))
    } else {
        None
    };

    tracing::info!(path = %settings.database.url, "database SQLite");
    let db = db::connect(&settings.database.url).await?;
    db::migrate(&db).await?;
    db::users::seed_admin(&db, settings.auth.admin_default_password.as_deref()).await?;

    let qdrant = clients::qdrant_store::QdrantStore::new(
        &settings.qdrant.grpc_url,
        &settings.qdrant.collection,
    )
    .await
    .context("qdrant init")?;
    tracing::info!("qdrant pronto");

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
        state::AppState::new(settings, db, embeddings, Arc::new(qdrant), eullm, live_bench);
    // Handle separato: app_state viene consumato da api::router() sotto, ma
    // serve ancora dopo la select! di shutdown per scrivere il report.
    let live_bench_for_shutdown = app_state.live_bench.clone();

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

    // --bench-live: scrive il report aggregato della sessione appena
    // conclusa, se è stata registrata almeno un'ingestione o una query.
    if let Some(rec) = &live_bench_for_shutdown {
        match rec.write_report() {
            Ok(Some(path)) => tracing::info!(path = %path.display(), "report benchmark live scritto"),
            Ok(None) => tracing::info!(
                "--bench-live: nessuna ingestione/query registrata in questa sessione, nessun report scritto"
            ),
            Err(e) => tracing::warn!(error = %e, "scrittura report benchmark live fallita"),
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

async fn load_embedding(settings: &config::Settings) -> Result<clients::embeddings::EmbeddingService> {
    let model_id = settings.embeddings.model_id.clone();
    let require_gpu = settings.embeddings.require_gpu;
    let swap_during_ingestion = settings.embeddings.swap_during_ingestion;
    tokio::task::spawn_blocking(move || {
        if swap_during_ingestion {
            // Riposo su CPU: la VRAM resta libera per eullm finché non
            // parte un'ingestione (vedi AppState::swap_embeddings_to_gpu).
            // require_gpu non governa più il boot in questa modalità — vedi
            // EmbeddingsSettings::swap_during_ingestion.
            clients::embeddings::EmbeddingService::load_cpu_parked(&model_id)
        } else {
            clients::embeddings::EmbeddingService::load(&model_id, require_gpu)
        }
    })
    .await
    .context("join embedding load")?
    .context("embedding service load")
}

/// Se bootstrap ha avviato eullm con un path GGUF locale (manifest, nessun
/// model_override), usa lo STESSO path come "model" nelle richieste — eullm
/// lo accetta direttamente (vedi ProcessGuard), niente registrazione
/// necessaria. Se invece è stato avviato con un model_override (es. un
/// riferimento hf.co), quel riferimento NON è accettato tale e quale da
/// /api/generate — eullm lo normalizza in un nome canonico diverso per il
/// proprio registry interno (osservato: "hf.co/bartowski/Qwen_Qwen3.6-35B-
/// A3B-GGUF:Q4_K_M" in avvio → "qwen_qwen3.6-35b-a3b-gguf-q4_k_m" in
/// /api/tags — l'originale dà 500 "Model ... not found"), quindi si
/// interroga /api/tags per il nome vero invece di indovinare la regola di
/// normalizzazione (vedi bootstrap::fetch_active_model_name). Fallback sul
/// path di lancio se la query fallisce. Se eullm è esterno
/// (manage_subprocesses=false) usa Settings.eullm.model così com'è.
async fn build_eullm_client(
    settings: &config::Settings,
    guard: &bootstrap::ProcessGuard,
) -> clients::eullm::EullmClient {
    let eullm_model = match &guard.eullm_model_path {
        Some(path) if settings.eullm.model_override.is_some() => {
            bootstrap::fetch_active_model_name(&settings.eullm.url)
                .await
                .unwrap_or_else(|| path.display().to_string())
        }
        Some(path) => path.display().to_string(),
        None => settings.eullm.model.clone(),
    };

    clients::eullm::EullmClient::new(
        settings.eullm.url.clone(),
        eullm_model,
        settings.eullm.num_ctx,
        settings.eullm.num_predict,
        settings.eullm.repeat_penalty,
        settings.eullm.keep_alive,
    )
}

/// Forza il caricamento del modello in VRAM prima di servire traffico reale
/// (parità Python: llm_client.py::warmup(), "avoid timeout on first query").
/// /api/tags (già atteso in bootstrap) conferma solo che il processo eullm è
/// in ascolto, NON che il modello sia caricato in memoria — se il caricamento
/// di eullm è lazy, senza questo passo la prima query reale paga il
/// cold-start e può tornare vuota o incompleta.
async fn warmup_eullm(eullm: &clients::eullm::EullmClient) {
    tracing::info!("warmup eullm…");
    match eullm.invoke("hi").await {
        Ok(a) if a.trim().is_empty() => tracing::warn!(
            "eullm warmup: risposta vuota — il modello potrebbe non essere pronto (vedi eullm stesso)"
        ),
        Ok(_) => tracing::info!("eullm warmup completato, modello in VRAM"),
        Err(e) => tracing::warn!(error = %e, "eullm warmup fallito (si caricherà alla prima query reale)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_flag_detects_short_and_long_form() {
        assert!(is_version_flag(&args(&["i3k-rag-engine", "-V"])));
        assert!(is_version_flag(&args(&["i3k-rag-engine", "--version"])));
    }

    #[test]
    fn version_flag_absent_on_normal_startup() {
        assert!(!is_version_flag(&args(&["i3k-rag-engine"])));
        assert!(!is_version_flag(&args(&[])));
    }

    #[test]
    fn version_flag_case_sensitive_lowercase_v_is_not_version() {
        // -v (minuscola) non è riservata da questo binario oggi, ma non deve
        // essere confusa con -V — se in futuro -v diventa "verbose" o simile,
        // questo test impedisce che venga silenziosamente trattato come -V.
        assert!(!is_version_flag(&args(&["i3k-rag-engine", "-v"])));
    }
}
