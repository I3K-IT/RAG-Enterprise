//! Startup bootstrap: scarica eullm + GGUF, poi avvia eullm in serving.
//!
//! Strategia download:
//!  - HTTP Range multiconnessione (N chunk paralleli) con progress % + ETA.
//!  - I GGUF vengono scaricati direttamente da HuggingFace — nessun doppio download.
//!  - eullm viene avviato con il PATH del file (non il nome modello), così non serve
//!    creare manifest.json manualmente.
//!
//! Flusso:
//!  1. eullm già in ascolto → skip.
//!  2. Trova o scarica il binario eullm (parallel, progress).
//!  3. Scarica Qwen3-14B-Q4_K_M.gguf se assente (parallel, progress).
//!  4. Scarica Qwen3-8B-Q4_K_M.gguf  se assente (parallel, progress).
//!  5. `eullm run <path-14b> --fit --cli` in background.
//!  6. Attendi /api/tags ready.

use anyhow::{bail, Context, Result};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::process::Command;

// ── Costanti ──────────────────────────────────────────────────────────────────

const EULLM_REPO: &str = "eullm/eullm";
const EULLM_ASSET: &str = "eullm-linux-x64-cuda-12.8";
const EULLM_FALLBACK_VERSION: &str = "0.6.6";

// URL base per i file del modello embedding (bge-m3).
// Cambia in "https://i3k.dev/models/bge-m3" quando i file sono caricati su i3k.dev.
const BGE_M3_BASE_URL: &str = "https://huggingface.co/BAAI/bge-m3/resolve/main";

struct ModelInfo {
    /// Nome logico (per logging e path della cartella in ~/.eullm/models/)
    name: &'static str,
    /// Filename del GGUF
    file: &'static str,
    /// URL HuggingFace (resolve/main)
    url: &'static str,
}

const MODEL_14B: ModelInfo = ModelInfo {
    name: "qwen3-14b",
    file: "Qwen3-14B-Q4_K_M.gguf",
    url: "https://huggingface.co/unsloth/Qwen3-14B-GGUF/resolve/main/Qwen3-14B-Q4_K_M.gguf",
};
const MODEL_8B: ModelInfo = ModelInfo {
    name: "qwen3-8b",
    file: "Qwen3-8B-Q4_K_M.gguf",
    url: "https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
};

/// Connessioni parallele per ogni download (split Range).
const DOWNLOAD_CONNECTIONS: usize = 8;
/// Buffer per scrittura su disco: 1 MB per non fare spawn_blocking a ogni chunk HTTP.
const WRITE_BUFFER_BYTES: usize = 1 << 20;

const READY_TIMEOUT_SECS: u64 = 600;
const POLL_INTERVAL_MS: u64 = 2_000;
const PROBE_TIMEOUT_SECS: u64 = 3;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn ensure_ready(eullm_url: &str, embedding_model_id: &str) -> Result<()> {
    // Embedding model: sempre necessario, scarica prima di avviare eullm
    ensure_embedding_model(embedding_model_id).await?;

    if probe_api(eullm_url).await {
        tracing::info!("eullm già in ascolto su {eullm_url}");
        return Ok(());
    }

    // 1. Binario eullm
    let bin = find_or_install_eullm().await?;

    // 2. Modello principale (inference + ingestione)
    let gguf_14b = ensure_gguf(&MODEL_14B).await?;

    // 3. Modello extraction SQL (schedulato / on-demand)
    let _gguf_8b = ensure_gguf(&MODEL_8B).await?;

    // 4. Avvia eullm con il path del GGUF (--cli: no chat embedding; --fit: auto VRAM)
    spawn_eullm_background(&bin, &gguf_14b);

    // 5. Attendi API ready
    wait_for_api(eullm_url, READY_TIMEOUT_SECS).await
}

// ── GGUF: controlla / scarica ─────────────────────────────────────────────────

async fn ensure_gguf(m: &ModelInfo) -> Result<PathBuf> {
    let dest = gguf_path(m)?;

    if dest.exists() {
        tracing::info!("{}: già in cache ({})", m.name, dest.display());
        return Ok(dest);
    }

    // Crea la directory del modello
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("mkdir {}", parent.display()))?;
    }

    tracing::info!("{}: avvio download da HuggingFace → {}", m.name, dest.display());
    parallel_download(m.url, &dest, m.file, DOWNLOAD_CONNECTIONS).await?;
    Ok(dest)
}

fn gguf_path(m: &ModelInfo) -> Result<PathBuf> {
    let home = std::env::var("HOME").context("$HOME non definita")?;
    Ok(PathBuf::from(home)
        .join(".eullm/models")
        .join(m.name)
        .join(m.file))
}

// ── Binario eullm: controlla / scarica ───────────────────────────────────────

async fn find_or_install_eullm() -> Result<PathBuf> {
    let candidates = {
        let mut v = vec![PathBuf::from("./eullm")];
        if let Ok(home) = std::env::var("HOME") {
            v.push(PathBuf::from(&home).join(".local/bin/eullm"));
        }
        v
    };

    for p in &candidates {
        if p.exists() {
            tracing::info!("eullm trovato: {}", p.display());
            return Ok(p.clone());
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

    download_eullm().await
}

async fn download_eullm() -> Result<PathBuf> {
    let version = latest_eullm_version().await;
    let url = format!(
        "https://github.com/{EULLM_REPO}/releases/download/EuLLM-v{version}/{EULLM_ASSET}"
    );
    let dest = PathBuf::from("./eullm");

    tracing::info!("download eullm v{version}: {url}");
    parallel_download(&url, &dest, "eullm", DOWNLOAD_CONNECTIONS).await?;

    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
        .await
        .context("chmod +x ./eullm")?;

    tracing::info!("eullm v{version} installato: {}", dest.display());
    Ok(dest)
}

async fn latest_eullm_version() -> String {
    let url = format!("https://api.github.com/repos/{EULLM_REPO}/releases/latest");
    let ver: Option<String> = async {
        let v: serde_json::Value = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", "i3k-rag-engine")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        v["tag_name"]
            .as_str()?
            .strip_prefix("EuLLM-v")
            .map(str::to_owned)
    }
    .await;
    ver.unwrap_or_else(|| {
        tracing::warn!(fallback = EULLM_FALLBACK_VERSION, "versione eullm non verificabile");
        EULLM_FALLBACK_VERSION.to_owned()
    })
}

// ── Embedding model: controlla / scarica ─────────────────────────────────────

async fn ensure_embedding_model(model_id: &str) -> Result<()> {
    if crate::clients::embeddings::find_model_in_cache(model_id).is_some() {
        tracing::info!("{model_id}: già in cache locale");
        return Ok(());
    }

    let dest = crate::clients::embeddings::download_target_dir(model_id);
    tracing::info!("{model_id}: non in cache, avvio download → {}", dest.display());
    tokio::fs::create_dir_all(&dest)
        .await
        .with_context(|| format!("mkdir {}", dest.display()))?;

    let client = reqwest::Client::builder()
        .user_agent("i3k-rag-engine/0.1")
        .timeout(Duration::from_secs(300))
        .build()
        .context("reqwest client")?;

    let base = BGE_M3_BASE_URL;

    // File di configurazione piccoli: download diretto
    for name in &["config.json", "tokenizer.json"] {
        let path = dest.join(name);
        if !path.exists() {
            tracing::info!("{model_id}/{name}: download…");
            download_small_file(&client, &format!("{base}/{name}"), &path)
                .await
                .with_context(|| format!("download {name}"))?;
        }
    }

    // Pesi: download parallelo (8 connessioni) con temp file + rename atomico
    let single = dest.join("model.safetensors");
    if !single.exists() {
        let tmp = dest.join("model.safetensors.tmp");
        let _ = tokio::fs::remove_file(&tmp).await; // pulisce residui precedenti
        match parallel_download(
            &format!("{base}/model.safetensors"),
            &tmp,
            &format!("{model_id}/model.safetensors"),
            DOWNLOAD_CONNECTIONS,
        )
        .await
        {
            Ok(()) => {
                tokio::fs::rename(&tmp, &single)
                    .await
                    .context("rename model.safetensors")?;
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                tracing::warn!("model.safetensors non disponibile ({e:#}), provo sharded…");
                download_sharded_model(&client, &base, &dest).await?;
            }
        }
    }

    tracing::info!("{model_id}: download completato");
    Ok(())
}

async fn download_sharded_model(client: &reqwest::Client, base: &str, dest: &Path) -> Result<()> {
    let idx_name = "model.safetensors.index.json";
    let idx_path = dest.join(idx_name);
    download_small_file(client, &format!("{base}/{idx_name}"), &idx_path)
        .await
        .context("download safetensors index")?;

    let idx: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(&idx_path)?)
            .context("parse safetensors.index.json")?;

    let mut shards: Vec<String> = idx["weight_map"]
        .as_object()
        .context("weight_map mancante")?
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
            let tmp = dest.join(format!("{shard}.tmp"));
            let _ = tokio::fs::remove_file(&tmp).await;
            parallel_download(
                &format!("{base}/{shard}"),
                &tmp,
                &shard,
                DOWNLOAD_CONNECTIONS,
            )
            .await
            .with_context(|| format!("download shard {shard}"))?;
            tokio::fs::rename(&tmp, &path)
                .await
                .with_context(|| format!("rename {shard}"))?;
        }
    }
    Ok(())
}

async fn download_small_file(client: &reqwest::Client, url: &str, dest: &Path) -> Result<()> {
    let bytes = client
        .get(url)
        .send()
        .await
        .context("GET")?
        .error_for_status()
        .context("HTTP")?
        .bytes()
        .await
        .context("bytes")?;
    tokio::fs::write(dest, &bytes).await.context("write")?;
    Ok(())
}

// ── Download parallelo multi-chunk ────────────────────────────────────────────

/// Scarica `url` in `dest` usando `n` connessioni parallele con Range requests.
/// Stampa progress % + ETA ogni 3 secondi via tracing::info!.
/// Fallback a singola connessione se il server non supporta Range.
async fn parallel_download(url: &str, dest: &Path, display_name: &str, n: usize) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3600))
        .build()?;

    // Probe GET (non HEAD): segue i redirect CDN e restituisce l'URL finale con
    // content-length corretto. I CDN con URL pre-firmati (HuggingFace LFS / S3)
    // spesso non rispondono correttamente alle HEAD — content-length mancante o 0.
    let probe = client.get(url).send().await.context("probe GET")?;
    let final_url = probe.url().to_string();

    let total: u64 = probe
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let accepts_ranges = probe
        .headers()
        .get("accept-ranges")
        .map(|v| v != "none")
        .unwrap_or(true);

    if total == 0 || !accepts_ranges {
        tracing::info!("{display_name}: download streaming (Range non supportato)");
        download_streaming(probe, dest, display_name).await?;
        return Ok(());
    }
    drop(probe); // non scarichiamo il body — usiamo Range requests sull'URL finale

    // Pre-alloca il file
    {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(dest)
            .with_context(|| format!("open {}", dest.display()))?;
        f.set_len(total)?;
    }

    let file = Arc::new(
        std::fs::OpenOptions::new()
            .write(true)
            .open(dest)
            .with_context(|| format!("open write {}", dest.display()))?,
    );

    let downloaded = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    // Chunk boundaries
    let chunk_size = (total + n as u64 - 1) / n as u64;
    let ranges: Vec<(u64, u64)> = (0..n as u64)
        .map(|i| {
            let s = i * chunk_size;
            let e = (s + chunk_size - 1).min(total - 1);
            (s, e)
        })
        .collect();

    // Task progress
    let dl_ref = Arc::clone(&downloaded);
    let name_str = display_name.to_owned();
    let progress_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let done = dl_ref.load(Ordering::Relaxed);
            if done == 0 {
                continue;
            }
            let pct = done as f64 / total as f64 * 100.0;
            let elapsed = start.elapsed().as_secs_f64();
            let rate = done as f64 / elapsed; // byte/s
            let eta = if rate > 0.0 {
                fmt_eta(((total - done) as f64 / rate) as u64)
            } else {
                "…".to_owned()
            };
            tracing::info!(
                "{name_str}: {pct:.1}%  ({} / {})  ETA {eta}",
                fmt_bytes(done),
                fmt_bytes(total),
            );
            if done >= total {
                break;
            }
        }
    });

    // Chunk download tasks — usano l'URL finale (post-redirect) per evitare
    // un redirect per ogni Range request
    let mut tasks = Vec::with_capacity(n);
    for (chunk_start, chunk_end) in ranges {
        let client = client.clone();
        let url = final_url.clone();
        let file = Arc::clone(&file);
        let dl = Arc::clone(&downloaded);
        tasks.push(tokio::spawn(async move {
            download_chunk(client, url, file, chunk_start, chunk_end, dl).await
        }));
    }

    for t in tasks {
        t.await
            .context("join chunk task")?
            .context("chunk download")?;
    }

    progress_task.abort();
    tracing::info!(
        "{display_name}: completato  {}  in {}",
        fmt_bytes(total),
        fmt_eta(start.elapsed().as_secs()),
    );
    Ok(())
}

/// Streaming download (fallback quando il server non supporta Range o non fornisce content-length).
/// Mostra progress ogni 10%.
async fn download_streaming(
    resp: reqwest::Response,
    dest: &Path,
    display_name: &str,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await.context("crea file")?;
    let mut downloaded: u64 = 0;
    let mut stream = resp;
    let start = Instant::now();
    let mut last_pct = 0u64;
    while let Some(chunk) = stream.chunk().await.context("chunk")? {
        file.write_all(&chunk).await.context("write")?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            let pct = downloaded * 100 / total;
            if pct / 10 != last_pct / 10 {
                let elapsed = start.elapsed().as_secs_f64().max(0.001);
                let rate = downloaded as f64 / elapsed;
                let eta = fmt_eta(((total - downloaded) as f64 / rate) as u64);
                tracing::info!(
                    "{display_name}: {pct:.0}%  ({} / {})  ETA {eta}",
                    fmt_bytes(downloaded),
                    fmt_bytes(total),
                );
                last_pct = pct;
            }
        }
    }
    file.flush().await.context("flush")?;
    tracing::info!(
        "{display_name}: completato  {}  in {}",
        fmt_bytes(downloaded.max(total)),
        fmt_eta(start.elapsed().as_secs()),
    );
    Ok(())
}

/// Scarica il range `[start, end]` e lo scrive nel file alla posizione corretta.
/// Bufferizza le scritture a 1 MB per ridurre le chiamate spawn_blocking.
async fn download_chunk(
    client: reqwest::Client,
    url: String,
    file: Arc<std::fs::File>,
    start: u64,
    end: u64,
    downloaded: Arc<AtomicU64>,
) -> Result<()> {
    let mut resp = client
        .get(&url)
        .header("Range", format!("bytes={start}-{end}"))
        .send()
        .await
        .context("range GET")?;

    let mut buf: Vec<u8> = Vec::with_capacity(WRITE_BUFFER_BYTES);
    let mut write_offset = start;

    while let Some(chunk) = resp.chunk().await.context("chunk read")? {
        downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        buf.extend_from_slice(&chunk);

        if buf.len() >= WRITE_BUFFER_BYTES {
            let data = std::mem::take(&mut buf);
            let f = Arc::clone(&file);
            let off = write_offset;
            write_offset += data.len() as u64;
            tokio::task::spawn_blocking(move || f.write_all_at(&data, off))
                .await
                .context("spawn_blocking write")?
                .context("pwrite")?;
        }
    }

    // Flush remainder
    if !buf.is_empty() {
        let f = Arc::clone(&file);
        let off = write_offset;
        tokio::task::spawn_blocking(move || f.write_all_at(&buf, off))
            .await
            .context("spawn_blocking write (flush)")?
            .context("pwrite (flush)")?;
    }

    Ok(())
}

// ── Avvio eullm ───────────────────────────────────────────────────────────────

fn spawn_eullm_background(bin: &Path, model_path: &Path) {
    let bin = bin.to_owned();
    let model_path = model_path.to_owned();
    tokio::spawn(async move {
        tracing::info!("avvio eullm: {} {}", bin.display(), model_path.display());
        let result = Command::new(&bin)
            .arg("run")
            .arg(&model_path)
            .arg("--fit")
            .arg("--cli")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .await;
        match result {
            Ok(s) if !s.success() => tracing::error!(code = ?s.code(), "eullm terminato con errore"),
            Err(e) => tracing::error!(err = %e, "errore avvio processo eullm"),
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
    tracing::info!("attendo eullm su {url} (max {timeout_secs}s)");
    loop {
        if probe_api(url).await {
            tracing::info!("eullm API ready");
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("eullm non risponde dopo {timeout_secs}s");
        }
        if tick > 0 && tick % 15 == 0 {
            tracing::info!(elapsed_s = tick * 2, "ancora in attesa di eullm...");
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        tick += 1;
    }
}

// ── Formattatori ─────────────────────────────────────────────────────────────

fn fmt_bytes(b: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    if b >= GB {
        format!("{:.1} GB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.0} MB", b as f64 / MB as f64)
    } else {
        format!("{} KB", b >> 10)
    }
}

fn fmt_eta(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}
