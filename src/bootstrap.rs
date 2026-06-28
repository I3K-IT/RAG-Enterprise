//! Startup bootstrap: garantisce che eullm sia presente e avviato prima che il server parta.
//!
//! Flusso al primo avvio:
//!  1. Se eullm è già raggiungibile → skip.
//!  2. Trova il binario eullm (./eullm, ~/.local/bin/eullm, PATH).
//!     Se non trovato → scarica l'ultima release da GitHub.
//!  3. Pre-pull qwen3-8b se non in cache:
//!     - Avvia `eullm run qwen3-8b --fit` (scarica ~5 GB, poi carica)
//!     - Attende API ready
//!     - Killa eullm (solo download, non deve restare in serving)
//!  4. Avvia `eullm run qwen3-14b --fit` in background (scarica ~9 GB se assente)
//!  5. Attende API ready → server axum può partire.
//!
//! Modelli:
//!  - qwen3-14b  unsloth/Qwen3-14B-GGUF / Qwen3-14B-Q4_K_M.gguf  ~9 GB  (chat)
//!  - qwen3-8b   unsloth/Qwen3-8B-GGUF  / Qwen3-8B-Q4_K_M.gguf   ~5 GB  (extraction SQL)
//!
//! Cache locale: ~/.eullm/models/<model>/manifest.json

use anyhow::{bail, Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;

// ── Costanti ──────────────────────────────────────────────────────────────────

const EULLM_REPO: &str = "eullm/eullm";
const EULLM_ASSET: &str = "eullm-linux-x64-cuda-12.8";
const EULLM_FALLBACK_VERSION: &str = "0.6.6";

const MODEL_CHAT: &str = "qwen3-14b";
const MODEL_EXTRACT: &str = "qwen3-8b";

const READY_TIMEOUT_SECS: u64 = 600; // 10 min: primo avvio + download 9 GB
const POLL_INTERVAL_MS: u64 = 2_000;
const PROBE_TIMEOUT_SECS: u64 = 3;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Chiamata da main() prima di avviare il server axum.
/// Al termine: eullm è in ascolto su `eullm_url` con qwen3-14b caricato.
pub async fn ensure_ready(eullm_url: &str) -> Result<()> {
    // Già in esecuzione? (secondo avvio, o eullm gestito manualmente)
    if probe_api(eullm_url).await {
        tracing::info!("eullm già in ascolto su {eullm_url}");
        log_model_cache_status();
        return Ok(());
    }

    // Trova o scarica il binario eullm
    let bin = find_or_install_eullm().await?;

    // Pre-pull qwen3-8b se non in cache.
    // Avvia eullm-8b, attende il pull+load, poi lo killa.
    // Deve stare prima del 14b: non possono coesistere in VRAM.
    ensure_model_cached(&bin, MODEL_EXTRACT, eullm_url).await?;

    // Avvia eullm con qwen3-14b — rimane in serving per tutta la sessione.
    // `eullm run --fit` scarica il modello se non in cache.
    spawn_eullm_background(&bin, MODEL_CHAT);

    // Attendi che /api/tags risponda
    wait_for_api(eullm_url, READY_TIMEOUT_SECS).await
}

// ── Pre-pull modello ──────────────────────────────────────────────────────────

/// Se il modello non è in cache, avvia eullm per scaricarlo, poi lo ferma.
async fn ensure_model_cached(bin: &Path, model: &str, eullm_url: &str) -> Result<()> {
    if model_is_cached(model) {
        tracing::info!("{model} già in cache");
        return Ok(());
    }

    tracing::info!(
        model = %model,
        "modello non in cache — avvio pull tramite eullm (gli output di eullm compaiono qui sotto)"
    );

    let mut child = Command::new(bin)
        .arg("run")
        .arg(model)
        .arg("--fit")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit()) // l'utente vede il progresso pull
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn eullm run {model}"))?;

    // Attendi che il modello sia caricato e l'API sia up
    if let Err(e) = wait_for_api(eullm_url, READY_TIMEOUT_SECS).await {
        child.kill().await.ok();
        child.wait().await.ok();
        return Err(e);
    }

    // Modello scaricato e caricato → ferma eullm
    tracing::info!("{model} pronto — fermo eullm per liberare VRAM");
    child.kill().await.context("kill eullm dopo pull {model}")?;
    child.wait().await.ok();

    // Piccola pausa per rilascio porta
    tokio::time::sleep(Duration::from_secs(2)).await;

    tracing::info!("{model} in cache");
    Ok(())
}

// ── Serving eullm (background) ────────────────────────────────────────────────

fn spawn_eullm_background(bin: &Path, model: &str) {
    let bin = bin.to_owned();
    let model = model.to_owned();
    tokio::spawn(async move {
        tracing::info!(model = %model, "avvio eullm in serving");
        let result = Command::new(&bin)
            .arg("run")
            .arg(&model)
            .arg("--fit")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .await;
        match result {
            Ok(s) if !s.success() => tracing::error!(code = ?s.code(), "eullm terminato con errore"),
            Err(e) => tracing::error!(err = %e, "errore processo eullm"),
            Ok(_) => tracing::info!("eullm terminato"),
        }
    });
}

// ── Attesa API ────────────────────────────────────────────────────────────────

async fn probe_api(url: &str) -> bool {
    reqwest::Client::new()
        .get(format!("{url}/api/tags"))
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn wait_for_api(url: &str, timeout_secs: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut tick: u32 = 0;
    tracing::info!("attendo eullm su {url} (max {timeout_secs}s)...");
    loop {
        if probe_api(url).await {
            tracing::info!("eullm API ready");
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("eullm non risponde dopo {timeout_secs}s — controlla i log di eullm");
        }
        if tick > 0 && tick % 15 == 0 {
            tracing::info!(elapsed_s = tick * 2, "ancora in attesa di eullm...");
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        tick += 1;
    }
}

// ── Binario eullm ─────────────────────────────────────────────────────────────

async fn find_or_install_eullm() -> Result<PathBuf> {
    // Candidati in ordine di preferenza
    let mut candidates = vec![PathBuf::from("./eullm")];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join(".local/bin/eullm"));
    }

    for path in &candidates {
        if path.exists() {
            tracing::info!(path = %path.display(), "trovato binario eullm");
            return Ok(path.clone());
        }
    }

    // Controlla PATH
    if std::process::Command::new("eullm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        tracing::info!("eullm trovato nel PATH");
        return Ok(PathBuf::from("eullm"));
    }

    // Non trovato → scarica
    download_eullm().await
}

async fn download_eullm() -> Result<PathBuf> {
    let version = latest_eullm_version().await;
    let url = format!(
        "https://github.com/{EULLM_REPO}/releases/download/EuLLM-v{version}/{EULLM_ASSET}"
    );
    tracing::info!(version = %version, "scarico eullm (~100 MB): {url}");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(900))
        .build()?;

    let bytes = client
        .get(&url)
        .send()
        .await
        .context("download eullm: request fallita")?
        .bytes()
        .await
        .context("download eullm: lettura bytes fallita")?;

    let dest = PathBuf::from("./eullm");
    tokio::fs::write(&dest, &bytes)
        .await
        .context("salvataggio ./eullm")?;
    tokio::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
        .await
        .context("chmod +x ./eullm")?;

    tracing::info!(path = %dest.display(), "eullm v{version} installato");
    Ok(dest)
}

async fn latest_eullm_version() -> String {
    let url = format!("https://api.github.com/repos/{EULLM_REPO}/releases/latest");
    let version: Option<String> = async {
        let resp: serde_json::Value = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", "i3k-rag-engine")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        // "EuLLM-v0.6.6" → "0.6.6"
        resp["tag_name"]
            .as_str()?
            .strip_prefix("EuLLM-v")
            .map(str::to_owned)
    }
    .await;

    version.unwrap_or_else(|| {
        tracing::warn!(
            fallback = EULLM_FALLBACK_VERSION,
            "impossibile verificare ultima versione eullm"
        );
        EULLM_FALLBACK_VERSION.to_owned()
    })
}

// ── Cache modelli ─────────────────────────────────────────────────────────────

fn model_is_cached(model: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".eullm/models")
        .join(model)
        .join("manifest.json")
        .exists()
}

fn log_model_cache_status() {
    for model in [MODEL_CHAT, MODEL_EXTRACT] {
        if model_is_cached(model) {
            tracing::info!("{model}: in cache");
        } else {
            tracing::info!("{model}: non in cache (sarà scaricato al prossimo avvio)");
        }
    }
}
