//! Bootstrap: scarica, verifica (SHA256) e avvia i componenti (manifest.toml).
//!
//! Flusso:
//!  1. Carica manifest embedded (include_str!).
//!  2. Crea struttura directory {data_dir}/{bin,models,storage/qdrant,db,uploads,backups}.
//!  3. Pre-check spazio disco (2x margine sulle dimensioni totali mancanti).
//!  4. Per ogni componente: controlla presenza + sha256; se manca/errato → download atomico.
//!  5. Se manage_subprocesses=true: avvia qdrant ed eullm come processi figlio supervisionati.
//!  6. Attende API ready di entrambi.
//!  7. Ritorna ProcessGuard: al drop, i figli ricevono SIGKILL (kill_on_drop).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::process::Command;

use crate::config::Settings;

// ── Manifest ──────────────────────────────────────────────────────────────────

const MANIFEST_STR: &str = include_str!("../manifest.toml");

#[derive(Debug, Deserialize, Clone)]
struct Component {
    name: String,
    version: String,
    #[allow(dead_code)]
    kind: String,
    url: String,
    sha256: String,
    size: u64,
    dest: String,
    exec: bool,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    components: Vec<Component>,
}

fn load_manifest() -> Result<Manifest> {
    toml::from_str(MANIFEST_STR).context("manifest.toml non valido — bug di compilazione")
}

// ── Guardia processi supervisionati ───────────────────────────────────────────

/// Tiene in vita i processi figli. Al drop: SIGKILL (kill_on_drop=true).
pub struct ProcessGuard {
    _children: Vec<tokio::process::Child>,
}

// ── Costanti ──────────────────────────────────────────────────────────────────

const DOWNLOAD_CONNECTIONS: usize = 8;
const WRITE_BUFFER_BYTES: usize = 1 << 20; // 1 MB

const QDRANT_READY_TIMEOUT_SECS: u64 = 60;
const EULLM_READY_TIMEOUT_SECS: u64 = 600;
const POLL_INTERVAL_MS: u64 = 2_000;
const PROBE_TIMEOUT_SECS: u64 = 3;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn ensure_ready(settings: &Settings) -> Result<ProcessGuard> {
    let manifest = load_manifest()?;
    let data_dir = settings.data.data_path();

    // 1. Struttura directory
    for subdir in &["bin", "models", "storage/qdrant", "db", "uploads", "backups"] {
        tokio::fs::create_dir_all(data_dir.join(subdir))
            .await
            .with_context(|| format!("mkdir {}/{subdir}", data_dir.display()))?;
    }

    // 2. Pre-check spazio disco
    check_disk_space(&manifest, &data_dir)?;

    // 3. Scarica / verifica ogni componente
    for comp in &manifest.components {
        let dest = data_dir.join(&comp.dest);
        ensure_component(comp, &dest).await?;
    }

    let mut children: Vec<tokio::process::Child> = Vec::new();

    if settings.data.manage_subprocesses {
        // Qdrant
        let qdrant_url = &settings.qdrant.url;
        if probe_url(&format!("{qdrant_url}/healthz")).await {
            tracing::info!("qdrant già in ascolto su {qdrant_url}");
        } else {
            let qdrant_bin = component_path(&manifest, "qdrant", &data_dir)?;
            let qdrant_storage = data_dir.join("storage/qdrant");
            let child = spawn_qdrant(&qdrant_bin, &qdrant_storage)?;
            children.push(child);
            tracing::info!("qdrant avviato: {}", qdrant_bin.display());
        }

        // eullm
        let eullm_url = &settings.eullm.url;
        if probe_url(&format!("{eullm_url}/api/tags")).await {
            tracing::info!("eullm già in ascolto su {eullm_url}");
        } else {
            let eullm_bin = component_path(&manifest, "eullm", &data_dir)?;
            let gguf_14b = component_path(&manifest, "qwen3-14b", &data_dir)?;
            let child = spawn_eullm(&eullm_bin, &gguf_14b)?;
            children.push(child);
            tracing::info!("eullm avviato: {} {}", eullm_bin.display(), gguf_14b.display());
        }
    } else {
        tracing::info!("manage_subprocesses=false: attendo processi esterni");
    }

    // Attendi API (sempre — sia managed che external)
    wait_for_url(
        &format!("{}/healthz", settings.qdrant.url),
        QDRANT_READY_TIMEOUT_SECS,
        "qdrant",
    )
    .await?;
    wait_for_url(
        &format!("{}/api/tags", settings.eullm.url),
        EULLM_READY_TIMEOUT_SECS,
        "eullm",
    )
    .await?;

    Ok(ProcessGuard { _children: children })
}

// ── Componente: verifica / download atomico ───────────────────────────────────

async fn ensure_component(comp: &Component, dest: &Path) -> Result<()> {
    if dest.exists() {
        if verify_component(comp, dest).await? {
            tracing::info!("{} ({}): presente", comp.name, comp.version);
            return Ok(());
        }
        tracing::warn!("{}: verifica fallita, ri-scarico", comp.name);
        tokio::fs::remove_file(dest).await.ok();
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Download atomico: scarica in .partial, verifica sha256, rinomina
    let partial = dest.with_extension("partial");
    tokio::fs::remove_file(&partial).await.ok();

    parallel_download(&comp.url, &partial, &comp.name, DOWNLOAD_CONNECTIONS).await?;

    // Verifica sha256 dopo download
    if !comp.sha256.is_empty() {
        let expected = comp.sha256.clone();
        let p = partial.clone();
        let got = tokio::task::spawn_blocking(move || sha256_file(&p))
            .await
            .context("spawn_blocking sha256")?
            .context("sha256 calcolo")?;
        if got != expected {
            tokio::fs::remove_file(&partial).await.ok();
            bail!(
                "{}: sha256 errato dopo download\n  atteso:  {}\n  trovato: {}",
                comp.name,
                expected,
                got
            );
        }
        tracing::info!("{}: sha256 ok", comp.name);
    }

    tokio::fs::rename(&partial, dest)
        .await
        .with_context(|| format!("rename {}", dest.display()))?;

    if comp.exec {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .await
            .with_context(|| format!("chmod +x {}", dest.display()))?;
        tracing::info!("{}: chmod +x", comp.name);
    }

    Ok(())
}

/// Verifica un componente già su disco.
/// sha256 non vuoto → calcola e confronta il digest.
/// sha256 vuoto     → solo controllo esistenza (nessun sha256 nel manifest).
async fn verify_component(comp: &Component, dest: &Path) -> Result<bool> {
    if comp.sha256.is_empty() {
        return Ok(true);
    }

    let expected = comp.sha256.clone();
    let p = dest.to_owned();
    let name = comp.name.clone();
    let got = tokio::task::spawn_blocking(move || {
        tracing::info!("{name}: verifica sha256 in corso…");
        sha256_file(&p)
    })
    .await
    .context("spawn_blocking sha256 verify")?
    .context("sha256 verify")?;

    Ok(got == expected)
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("read")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ── Spazio disco ──────────────────────────────────────────────────────────────

fn check_disk_space(manifest: &Manifest, data_dir: &Path) -> Result<()> {
    struct Item<'a> {
        label: &'a str,
        size: u64,
    }

    let mut needed: Vec<Item> = Vec::new();
    for comp in &manifest.components {
        if comp.size > 0 && !data_dir.join(&comp.dest).exists() {
            needed.push(Item { label: &comp.name, size: comp.size });
        }
    }

    if needed.is_empty() {
        return Ok(());
    }

    let total: u64 = needed.iter().map(|i| i.size).sum();
    let free = free_space_bytes(data_dir);

    eprintln!();
    eprintln!("  Primo avvio — download necessari:");
    for item in &needed {
        eprintln!("    • {:<44}  {:>8}", item.label, fmt_bytes(item.size));
    }
    eprintln!("  ─────────────────────────────────────────────────────────────────");
    eprintln!(
        "  Totale stimato:  {:>8}   ·   Server: i3k.dev (Europa, IT)",
        fmt_bytes(total)
    );

    if free != u64::MAX {
        eprintln!("  Spazio libero:  {}", fmt_bytes(free));
        let required = total.saturating_mul(2);
        if free < required {
            eprintln!();
            bail!(
                "Spazio su disco insufficiente: {} liberi, {} necessari (margine 2x).",
                fmt_bytes(free),
                fmt_bytes(required)
            );
        }
    }
    eprintln!();
    Ok(())
}

fn free_space_bytes(path: &Path) -> u64 {
    use std::ffi::CString;
    let mut p = path.to_path_buf();
    loop {
        if p.exists() {
            break;
        }
        match p.parent() {
            Some(par) => p = par.to_path_buf(),
            None => return u64::MAX,
        }
    }
    let Ok(cpath) = CString::new(p.as_os_str().as_encoded_bytes()) else {
        return u64::MAX;
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) } == 0 {
        (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64)
    } else {
        u64::MAX
    }
}

// ── Avvio processi ────────────────────────────────────────────────────────────

fn component_path(manifest: &Manifest, name: &str, data_dir: &Path) -> Result<PathBuf> {
    manifest
        .components
        .iter()
        .find(|c| c.name == name)
        .map(|c| data_dir.join(&c.dest))
        .with_context(|| format!("componente '{name}' non trovato nel manifest"))
}

fn spawn_qdrant(bin: &Path, storage: &Path) -> Result<tokio::process::Child> {
    Command::new(bin)
        .env("QDRANT__STORAGE__STORAGE_PATH", storage)
        .env("QDRANT__SERVICE__HTTP_PORT", "6333")
        .env("QDRANT__SERVICE__GRPC_PORT", "6334")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("avvio qdrant: {}", bin.display()))
}

fn spawn_eullm(bin: &Path, model_path: &Path) -> Result<tokio::process::Child> {
    Command::new(bin)
        .arg("run")
        .arg(model_path)
        .arg("--fit")
        .arg("--cli")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("avvio eullm: {}", bin.display()))
}

// ── Attesa API ────────────────────────────────────────────────────────────────

async fn probe_url(url: &str) -> bool {
    reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn wait_for_url(url: &str, timeout_secs: u64, label: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut tick: u32 = 0;
    tracing::info!("attendo {label} su {url} (max {timeout_secs}s)");
    loop {
        if probe_url(url).await {
            tracing::info!("{label} pronto");
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("{label} non risponde dopo {timeout_secs}s — URL: {url}");
        }
        if tick > 0 && tick % 15 == 0 {
            tracing::info!(elapsed_s = tick * 2, "ancora in attesa di {label}…");
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        tick += 1;
    }
}

// ── Download parallelo multi-chunk ────────────────────────────────────────────

/// Scarica `url` in `dest` usando `n` connessioni Range parallele.
/// Progress su riga singola (TTY) o tracing (non-TTY).
/// Fallback a streaming se il server non supporta Range.
async fn parallel_download(url: &str, dest: &Path, display_name: &str, n: usize) -> Result<()> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(3600)).build()?;

    let probe = client.get(url).send().await.context("probe GET")?;
    if !probe.status().is_success() {
        bail!("HTTP {} — {display_name} ({url})", probe.status());
    }
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
        return download_streaming(probe, dest, display_name).await;
    }
    drop(probe);

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

    let chunk_size = (total + n as u64 - 1) / n as u64;
    let ranges: Vec<(u64, u64)> = (0..n as u64)
        .map(|i| {
            let s = i * chunk_size;
            let e = (s + chunk_size - 1).min(total - 1);
            (s, e)
        })
        .collect();

    use std::io::IsTerminal;
    let is_tty = std::io::stderr().is_terminal();
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
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            let rate = done as f64 / elapsed;
            let eta = if rate > 0.0 {
                fmt_eta(((total - done) as f64 / rate) as u64)
            } else {
                "…".to_owned()
            };
            if is_tty {
                use std::io::Write;
                eprint!(
                    "\r  {name_str}: {pct:.1}%  ({} / {})  {}/s  ETA {eta}     ",
                    fmt_bytes(done),
                    fmt_bytes(total),
                    fmt_bytes(rate as u64),
                );
                let _ = std::io::stderr().flush();
            } else {
                tracing::info!(
                    "{name_str}: {pct:.1}%  ({} / {})  ETA {eta}",
                    fmt_bytes(done),
                    fmt_bytes(total),
                );
            }
            if done >= total {
                break;
            }
        }
    });

    let mut tasks = Vec::with_capacity(n);
    for (cs, ce) in ranges {
        let client = client.clone();
        let url = final_url.clone();
        let file = Arc::clone(&file);
        let dl = Arc::clone(&downloaded);
        tasks.push(tokio::spawn(
            async move { download_chunk(client, url, file, cs, ce, dl).await },
        ));
    }

    for t in tasks {
        t.await.context("join chunk task")?.context("chunk download")?;
    }

    progress_task.abort();
    if is_tty {
        use std::io::Write;
        eprintln!(
            "\r  {display_name}: completato  {}  in {}                    ",
            fmt_bytes(total),
            fmt_eta(start.elapsed().as_secs())
        );
        let _ = std::io::stderr().flush();
    } else {
        tracing::info!(
            "{display_name}: completato  {}  in {}",
            fmt_bytes(total),
            fmt_eta(start.elapsed().as_secs()),
        );
    }
    Ok(())
}

async fn download_streaming(
    resp: reqwest::Response,
    dest: &Path,
    display_name: &str,
) -> Result<()> {
    use std::io::IsTerminal;
    use tokio::io::AsyncWriteExt;
    let is_tty = std::io::stderr().is_terminal();
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
            if is_tty {
                use std::io::Write;
                let elapsed = start.elapsed().as_secs_f64().max(0.001);
                let rate = downloaded as f64 / elapsed;
                let eta = fmt_eta(((total - downloaded) as f64 / rate) as u64);
                eprint!(
                    "\r  {display_name}: {pct}%  ({} / {})  {}/s  ETA {eta}     ",
                    fmt_bytes(downloaded),
                    fmt_bytes(total),
                    fmt_bytes(rate as u64),
                );
                let _ = std::io::stderr().flush();
            } else if pct / 10 != last_pct / 10 {
                let elapsed = start.elapsed().as_secs_f64().max(0.001);
                let rate = downloaded as f64 / elapsed;
                let eta = fmt_eta(((total - downloaded) as f64 / rate) as u64);
                tracing::info!(
                    "{display_name}: {pct}%  ({} / {})  ETA {eta}",
                    fmt_bytes(downloaded),
                    fmt_bytes(total),
                );
                last_pct = pct;
            }
        }
    }
    file.flush().await.context("flush")?;
    if is_tty {
        use std::io::Write;
        eprintln!(
            "\r  {display_name}: completato  {}  in {}                    ",
            fmt_bytes(downloaded.max(total)),
            fmt_eta(start.elapsed().as_secs())
        );
        let _ = std::io::stderr().flush();
    } else {
        tracing::info!(
            "{display_name}: completato  {}  in {}",
            fmt_bytes(downloaded.max(total)),
            fmt_eta(start.elapsed().as_secs()),
        );
    }
    Ok(())
}

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
        format!("{secs}s")
    }
}
