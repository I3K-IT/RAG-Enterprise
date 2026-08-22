//! Shared I3K RAG runtime — the reusable core behind the Community binary
//! (`src/main.rs`, a thin launcher calling [`run`]) and, per
//! `I3K_RAG_Pro_Open_Core_Architecture.md` in the `rag-enterprise-pro`
//! repository, the Pro binary too, via a pinned Git dependency on this
//! crate. Pro must depend on this library rather than forking it — any
//! core change is made here first, tested, merged, then Pro's pinned
//! revision is bumped (see that document's section 18).

pub mod api;
pub mod auth;
pub mod backup;
pub mod bench;
pub mod bootstrap;
pub mod clients;
pub mod config;
pub mod db;
pub mod documents;
pub mod error;
pub mod extensions;
pub mod observability;
pub mod rag;
pub mod state;

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// -V/--version: print and exit IMMEDIATELY, before any bootstrap work
/// (tracing, config, qdrant/eullm). It also includes BUILD_GIT_HASH from
/// build.rs, because the version alone cannot distinguish two builds made
/// from the same Cargo.toml.
pub fn is_version_flag(args: &[String]) -> bool {
    args.iter().any(|a| a == "-V" || a == "--version")
}

/// This crate's own `(CARGO_PKG_VERSION, BUILD_GIT_HASH)`, the same values
/// `-V`/`--version` prints. A function, not just a side-effecting print, so
/// a Pro launcher statically linking this crate via the pinned Git
/// dependency (`I3K_RAG_Pro_Open_Core_Architecture.md`, rag-enterprise-pro
/// private repo, section 9) can read Community's own version/commit at
/// runtime and fold it into its own `--version` output (that document's
/// section 19). Because Pro compiles this crate from its own pinned git
/// checkout, `BUILD_GIT_HASH` here resolves to Community's real pinned
/// commit — not Pro's — with no extra plumbing needed.
pub fn build_info() -> (&'static str, &'static str) {
    (env!("CARGO_PKG_VERSION"), env!("BUILD_GIT_HASH"))
}

/// Runs the full i3k-rag-engine server with Community's own defaults — no
/// extensions, no extra API routes. The Community binary's `main()` is just
/// `i3k_rag_engine::run().await`.
pub async fn run() -> Result<()> {
    run_with_extensions(extensions::ExtensionRegistry::default(), None).await
}

/// Same as [`run`], but for a Pro launcher (or a test) that needs to
/// register extensions and/or merge in extra API routes — see
/// `extensions::ExtensionRegistry` and `extensions::api`'s doc comment.
/// This is the exact sequence a Pro launcher calls, per
/// `I3K_RAG_Pro_Open_Core_Architecture.md` (rag-enterprise-pro, private
/// repo) section 11 — which is why it lives here instead of in main.rs.
pub async fn run_with_extensions(
    extensions: extensions::ExtensionRegistry,
    pro_router: Option<axum::Router<state::AppState>>,
) -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if is_version_flag(&args) {
        let (version, hash) = build_info();
        println!("i3k-rag-engine {version} ({hash})");
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

    // Makes data_dir available to the embeddings module (find_model_in_cache).
    std::env::set_var("I3K_DATA_DIR", settings.data.data_path());

    // Bootstrap: download the components, start qdrant (and eullm, order
    // below) and wait for the APIs to be ready. `guard` keeps the supervised
    // child processes alive (dropping it SIGKILLs them) and also exposes the
    // exact GGUF path eullm was started with, when we manage it.
    //
    // Startup order: embedding model FIRST, eullm AFTER. Unconditionally.
    //
    // From 0.6.80 eullm always sizes its GPU offload itself, no longer only
    // under --fit, and computes the budget from the *free* VRAM it reads with
    // cudaMemGetInfo at startup (free_vram * 0.97 - 640 MiB; see
    // VRAM_SAFETY_FRACTION and COMPUTE_BUFFER_RESERVE_BYTES in eullm's
    // source). So the only correct order is to load bge-m3 first: eullm then
    // sees the already-reduced VRAM and adapts.
    //
    // The reverse order, which used to be the default (eullm first, with a
    // warmup to force the real allocation), became a trap on 0.6.80: eullm
    // would take nearly all the free VRAM and the embedding model would find
    // none left — bge-m3 is 2.27 GB of weights alone. That is why the branch
    // was removed rather than kept as an option.
    //
    // _guard is used only for its Drop from here on (kill_on_drop of the child
    // processes); eullm's GGUF path has already been consumed by
    // build_eullm_client() before this tuple.
    let (_guard, mut embeddings, eullm) = {
        let (phase1, qdrant_children) = bootstrap::provision_and_start_qdrant(&settings).await?;
        let embeddings = load_embedding(&settings).await?;
        tracing::info!(
            device = embeddings.as_ref().map_or("eullm", |e| e.device_label()),
            "embedding service ready (before eullm, so its sizing sees the already-reduced VRAM)"
        );
        let guard = bootstrap::start_eullm(&settings, phase1, qdrant_children).await?;
        let eullm = build_eullm_client(&settings, &guard).await;
        warmup_eullm(&eullm).await;
        (guard, embeddings, eullm)
    };

    // --bench/--benchmark: reuse the same bootstrap (qdrant + eullm +
    // embeddings) as above, then exit straight away — no database, no HTTP
    // server. _guard stays in scope until run() returns, so qdrant and eullm
    // are still shut down properly once the benchmark ends.
    if let Some(bench_args) = bench::parse_args(&args) {
        tracing::info!(doc = %bench_args.doc_path.display(), "benchmark mode");
        return bench::run(&settings, &bench_args, embeddings.as_mut(), Arc::new(eullm)).await;
    }

    // --bench-live: the server and frontend start normally, but every real
    // ingestion and query is timed (see api/documents.rs, api/query.rs) and
    // accumulated here. The report is written on shutdown, below.
    let live_bench = if bench::live_mode_requested(&args) {
        tracing::info!(
            "--bench-live enabled: recording timings and hardware for every real ingestion and query in this session; the report is written on shutdown"
        );
        Some(Arc::new(bench::LiveRecorder::new(embeddings.as_ref(), &settings.eullm.model)))
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
    tracing::info!("qdrant ready");

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
            tracing::warn!(error = %e, "backup scheduler did not start");
        }
    });

    let app_state = state::AppState::new(
        settings, db, embeddings, Arc::new(qdrant), eullm, live_bench, extensions,
    );
    // Separate handle: app_state is consumed by api::router() below, but is
    // still needed after the shutdown select! in order to write the report.
    let live_bench_for_shutdown = app_state.live_bench.clone();

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!("server listening on {addr}");

    open_browser(port);

    let router = api::router(app_state, pro_router);

    // Graceful shutdown: SIGINT (Ctrl+C) or SIGTERM → drop guard → SIGKILL children.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
        tokio::select! {
            r = axum::serve(listener, router) => r?,
            _ = tokio::signal::ctrl_c() => { tracing::info!("SIGINT received, shutting down"); }
            _ = sigterm.recv() => { tracing::info!("SIGTERM received, shutting down"); }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            r = axum::serve(listener, router) => r?,
            _ = tokio::signal::ctrl_c() => { tracing::info!("SIGINT received, shutting down"); }
        }
    }

    // --bench-live: write the aggregated report for the session that has just
    // ended, provided at least one ingestion or query was recorded.
    if let Some(rec) = &live_bench_for_shutdown {
        match rec.write_report() {
            Ok(Some(path)) => tracing::info!(path = %path.display(), "live benchmark report written"),
            Ok(None) => tracing::info!(
                "--bench-live: no ingestion or query was recorded in this session, no report written"
            ),
            Err(e) => tracing::warn!(error = %e, "writing the live benchmark report failed"),
        }
    }

    // guard is dropped here → child processes are SIGKILLed
    Ok(())
}

fn open_browser(port: u16) {
    let url = format!("http://localhost:{port}");
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        tracing::info!("opening browser: {url}");
        let _ = spawn_open_url(&url);
    });
}

/// Launches the platform's "open this in the default application" command.
/// Best-effort: the caller only logs, never surfaces a failure — not being
/// able to auto-open a browser tab is not worth failing startup over.
#[cfg(target_os = "windows")]
fn spawn_open_url(url: &str) -> std::io::Result<std::process::Child> {
    // `start` is a cmd builtin, not its own executable — must go through cmd.
    // The empty "" is required: `start` treats the first quoted argument as
    // the window title, so without it a URL passed as the "title" slot is
    // silently not opened.
    std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
}
#[cfg(target_os = "macos")]
fn spawn_open_url(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open").arg(url).spawn()
}
#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_open_url(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open").arg(url).spawn()
}

async fn load_embedding(
    settings: &config::Settings,
) -> Result<Option<clients::embeddings::EmbeddingService>> {
    if settings.embeddings.ingestion_embedding == config::IngestionEmbedding::Eullm {
        // Both ingestion AND query embedding go through eullm's own
        // /api/embed in this mode (api/documents.rs, api/query.rs::prepare)
        // — Candle is never called, so it is not worth loading at all, not
        // even on CPU: no mmap, no weight copy, no VRAM claimed regardless
        // of require_gpu, and bootstrap does not download the ~3.2GB of
        // Candle bge-m3 weights either (see bootstrap::select_components).
        return Ok(None);
    }
    let model_id = settings.embeddings.model_id.clone();
    let require_gpu = settings.embeddings.require_gpu;
    let ingestion_embedding = settings.embeddings.ingestion_embedding;
    tokio::task::spawn_blocking(move || {
        if ingestion_embedding == config::IngestionEmbedding::CandleGpu {
            // At rest on CPU: the VRAM stays free for eullm until an
            // ingestion begins (see AppState::swap_embeddings_to_gpu).
            // require_gpu no longer governs startup in this mode — see
            // IngestionEmbedding::CandleGpu.
            clients::embeddings::EmbeddingService::load_cpu_parked(&model_id)
        } else {
            // Off: candle is never swapped, require_gpu governs this one
            // permanent load as always.
            clients::embeddings::EmbeddingService::load(&model_id, require_gpu)
        }
    })
    .await
    .context("join embedding load")?
    .context("embedding service load")
    .map(Some)
}

/// If the bootstrap started eullm with a local GGUF path (from the manifest,
/// no model_override), the SAME path is used as "model" in requests: eullm
/// accepts it directly (see ProcessGuard), with no registration needed.
///
/// If it was started with a model_override instead — an hf.co reference, say —
/// that reference is NOT accepted verbatim by /api/generate: eullm normalises
/// it into a different canonical name for its own internal registry. Observed:
/// "hf.co/bartowski/Qwen_Qwen3.6-35B-A3B-GGUF:Q4_K_M" at startup becomes
/// "qwen_qwen3.6-35b-a3b-gguf-q4_k_m" in /api/tags, and the original returns a
/// 500 "Model ... not found". So we ask /api/tags for the real name rather
/// than trying to guess the normalisation rule (see
/// bootstrap::fetch_active_model_name), falling back to the launch path if the
/// query fails. When eullm is external (manage_subprocesses=false),
/// Settings.eullm.model is used as-is.
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
        settings.eullm.repeat_last_n,
        settings.eullm.keep_alive,
    )
}

/// Forces the model into VRAM before serving real traffic, so the first
/// query does not time out on a cold start.
/// /api/tags, already awaited during bootstrap, only confirms that the eullm
/// process is listening, NOT that the model is loaded in memory. If eullm
/// loads lazily, without this step the first real query pays the cold start
/// and can come back empty or truncated.
async fn warmup_eullm(eullm: &clients::eullm::EullmClient) {
    tracing::info!("warming eullm up…");
    match eullm.invoke("hi").await {
        Ok(a) if a.trim().is_empty() => tracing::warn!(
            "eullm warmup: empty response — the model may not be ready (check eullm itself)"
        ),
        Ok(_) => tracing::info!("eullm warmup complete, model resident in VRAM"),
        Err(e) => tracing::warn!(error = %e, "eullm warmup failed (it will load on the first real query)"),
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
    fn build_info_matches_what_the_version_flag_prints() {
        let (version, hash) = build_info();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert_eq!(hash, env!("BUILD_GIT_HASH"));
        assert!(!hash.is_empty(), "build.rs must always set BUILD_GIT_HASH, even to \"unknown\"");
    }

    #[test]
    fn version_flag_case_sensitive_lowercase_v_is_not_version() {
        // Lowercase -v is not reserved by this binary today, but it must not
        // be confused with -V. If -v ever becomes "verbose" or similar, this
        // test stops it from being silently treated as -V.
        assert!(!is_version_flag(&args(&["i3k-rag-engine", "-v"])));
    }
}
