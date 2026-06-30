//! Bootstrap: scarica, verifica (SHA256) e avvia i componenti (manifest.toml).
//!
//! Manifest (embedded):
//!  - `[[component]]` con kind="model" (universale) o kind="binary" (per-target).
//!  - dest contiene il placeholder "{data}" → risolto a runtime con data_dir.
//!  - sha256 verificato su OGNI download. Per file già presenti: stamp file
//!    ({dest}.sha2) — evita di ricalcolare sha256 di file da GB a ogni avvio.
//!
//! Target detection (compile-time):
//!  - linux-x86_64:       qdrant (sempre su Linux x86_64)
//!  - linux-x86_64-cuda:  eullm  (solo con --features cuda)
//!
//! Flusso:
//!  1. Carica manifest + rileva target.
//!  2. Crea struttura directory in data_dir.
//!  3. Pre-check disco = somma size dei componenti mancanti.
//!  4. Scarica/verifica ogni componente selezionato.
//!  5. Se manage_subprocesses=true: avvia qdrant + eullm (kill_on_drop).
//!  6. Attende /healthz (qdrant) e /api/tags (eullm).
//!  7. Ritorna ProcessGuard — al drop i figli ricevono SIGKILL.

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
    #[serde(default)]
    version: Option<String>,
    #[allow(dead_code)]
    kind: String,
    /// Solo per i binari. None = modello (scaricato sempre).
    #[serde(default)]
    target: Option<String>,
    url: String,
    sha256: String,
    size: u64,
    /// Percorso con placeholder "{data}" (risolto a runtime).
    dest: String,
    exec: bool,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    component: Vec<Component>,
}

fn load_manifest() -> Result<Manifest> {
    toml::from_str(MANIFEST_STR).context("manifest.toml non valido — bug di compilazione")
}

// ── Target detection (runtime) ────────────────────────────────────────────────

/// Rileva i target supportati a runtime (non a compile-time).
/// CUDA: controlla /dev/nvidia0 — esiste se il driver NVIDIA è caricato.
/// Così funziona sia con `cargo build` sia con `cargo build --features cuda`.
fn current_targets() -> Vec<&'static str> {
    #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
    return vec![];

    let mut t = vec!["linux-x86_64"];
    if std::path::Path::new("/dev/nvidia0").exists() {
        t.push("linux-x86_64-cuda");
        tracing::debug!("CUDA rilevata (/dev/nvidia0 presente) — target linux-x86_64-cuda abilitato");
    }
    t
}

fn component_selected(comp: &Component, targets: &[&str]) -> bool {
    match &comp.target {
        None => true, // modello — sempre
        Some(t) => targets.contains(&t.as_str()),
    }
}

// ── Risoluzione path ──────────────────────────────────────────────────────────

fn resolve_dest(dest: &str, data_dir: &Path) -> PathBuf {
    PathBuf::from(dest.replace("{data}", &data_dir.display().to_string()))
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
    let targets = current_targets();

    // Struttura directory
    for subdir in &["bin", "models", "storage/qdrant", "db", "uploads", "backups"] {
        tokio::fs::create_dir_all(data_dir.join(subdir))
            .await
            .with_context(|| format!("mkdir {}/{subdir}", data_dir.display()))?;
    }

    // Pre-check spazio disco
    check_disk_space(&manifest, &data_dir, &targets)?;

    // Scarica/verifica ogni componente selezionato
    for comp in manifest.component.iter().filter(|c| component_selected(c, &targets)) {
        let dest = resolve_dest(&comp.dest, &data_dir);
        ensure_component(comp, &dest).await?;
    }

    let mut children: Vec<tokio::process::Child> = Vec::new();

    if settings.data.manage_subprocesses {
        // Qdrant
        let qdrant_url = &settings.qdrant.url;
        if probe_url(&format!("{qdrant_url}/healthz")).await {
            tracing::info!("qdrant già in ascolto su {qdrant_url}");
        } else {
            match find_component(&manifest, "qdrant", &data_dir, &targets) {
                Some(bin) => {
                    let storage = data_dir.join("storage/qdrant");
                    children.push(spawn_qdrant(&bin, &storage)?);
                    tracing::info!("qdrant avviato: {}", bin.display());
                }
                None => tracing::warn!("qdrant non selezionato per questa piattaforma"),
            }
        }

        // eullm — decisione di avvio basata su presenza su disco, non sul target.
        // Il target filtra i DOWNLOAD (non scaricare CUDA binary su CPU);
        // ma se il file c'è, lo avviamo — il RAG dipende da eullm.
        match (
            find_by_name(&manifest, "eullm", &data_dir),
            find_by_name(&manifest, "qwen3-14b", &data_dir),
        ) {
            (Some(bin), Some(gguf)) => {
                kill_stale_process(&bin).await;
                children.push(spawn_eullm(&bin, &gguf)?);
                tracing::info!("eullm avviato: {} {}", bin.display(), gguf.display());
            }
            _ => tracing::warn!(
                "eullm o qwen3-14b non trovati in {} — RAG senza LLM",
                data_dir.display()
            ),
        }
    } else {
        tracing::info!("manage_subprocesses=false — processi esterni attesi");
    }

    // Attendi API (sempre)
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
            let ver = comp.version.as_deref().map(|v| format!(" ({v})")).unwrap_or_default();
            tracing::info!("{}{ver}: presente e verificato", comp.name);
            return Ok(());
        }
        tracing::warn!("{}: verifica sha256 fallita, ri-scarico", comp.name);
        tokio::fs::remove_file(dest).await.ok();
        remove_stamp(dest).await;
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Download atomico: .partial → verifica sha256 → rename → stamp
    let partial = dest.with_extension("partial");
    tokio::fs::remove_file(&partial).await.ok();

    parallel_download(&comp.url, &partial, &comp.name, DOWNLOAD_CONNECTIONS).await?;

    // SHA256 dopo download (obbligatorio)
    {
        let expected = comp.sha256.clone();
        let p = partial.clone();
        let name = comp.name.clone();
        let got = tokio::task::spawn_blocking(move || {
            tracing::info!("{name}: verifica sha256 post-download…");
            sha256_file(&p)
        })
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
        // Rinomina atomico
        tokio::fs::rename(&partial, dest)
            .await
            .with_context(|| format!("rename {}", dest.display()))?;
        // Scrivi stamp
        write_stamp(dest, &got).await;
    }

    if comp.exec {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .await
            .with_context(|| format!("chmod +x {}", dest.display()))?;
    }

    tracing::info!("{}: installato in {}", comp.name, dest.display());
    Ok(())
}

/// Verifica un componente già presente su disco.
///
/// Fast path: se esiste lo stamp `{dest}.sha2` con il digest atteso → ok.
/// Slow path: ricalcola sha256 (può richiedere decine di secondi su file grandi).
///            Se ok → scrive lo stamp per i prossimi avvii.
async fn verify_component(comp: &Component, dest: &Path) -> Result<bool> {
    // Fast path: stamp file
    let stamp_path = stamp_path(dest);
    if stamp_path.exists() {
        if let Ok(stamped) = tokio::fs::read_to_string(&stamp_path).await {
            if stamped.trim() == comp.sha256 {
                return Ok(true);
            }
        }
    }

    // Slow path: calcola sha256
    let expected = comp.sha256.clone();
    let p = dest.to_owned();
    let name = comp.name.clone();
    let got = tokio::task::spawn_blocking(move || {
        tracing::info!("{name}: verifica sha256 (prima verifica, può richiedere tempo)…");
        sha256_file(&p)
    })
    .await
    .context("spawn_blocking sha256 verify")?
    .context("sha256 verify")?;

    if got == expected {
        write_stamp(dest, &got).await;
        Ok(true)
    } else {
        tracing::warn!(
            "{}: sha256 mismatch — atteso {} trovato {}",
            comp.name,
            &expected[..8],
            &got[..8]
        );
        Ok(false)
    }
}

fn stamp_path(dest: &Path) -> PathBuf {
    let mut p = dest.to_owned().into_os_string();
    p.push(".sha2");
    PathBuf::from(p)
}

async fn write_stamp(dest: &Path, hash: &str) {
    let _ = tokio::fs::write(stamp_path(dest), hash).await;
}

async fn remove_stamp(dest: &Path) {
    let _ = tokio::fs::remove_file(stamp_path(dest)).await;
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 256 * 1024]; // 256 KB per read
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

fn check_disk_space(manifest: &Manifest, data_dir: &Path, targets: &[&str]) -> Result<()> {
    struct Item<'a> {
        label: &'a str,
        size: u64,
    }

    let mut needed: Vec<Item> = Vec::new();
    for comp in manifest.component.iter().filter(|c| component_selected(c, targets)) {
        let dest = resolve_dest(&comp.dest, data_dir);
        if comp.size > 0 && !dest.exists() {
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
        "  Totale:  {:>8}   ·   Server: i3k.dev (Europa, IT)",
        fmt_bytes(total)
    );

    if free != u64::MAX {
        eprintln!("  Spazio libero:  {}", fmt_bytes(free));
        if free < total {
            eprintln!();
            bail!(
                "Spazio su disco insufficiente: {} liberi, {} necessari.",
                fmt_bytes(free),
                fmt_bytes(total)
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

/// Cerca per nome E target (usato per decidere cosa scaricare).
fn find_component(manifest: &Manifest, name: &str, data_dir: &Path, targets: &[&str]) -> Option<PathBuf> {
    manifest
        .component
        .iter()
        .filter(|c| component_selected(c, targets))
        .find(|c| c.name == name)
        .map(|c| resolve_dest(&c.dest, data_dir))
}

/// Cerca per nome senza filtro target (usato per decidere cosa AVVIARE).
/// Il binario potrebbe essere già su disco anche se il target non combacia
/// (es. scaricato in una sessione precedente, o trasferito manualmente).
fn find_by_name(manifest: &Manifest, name: &str, data_dir: &Path) -> Option<PathBuf> {
    manifest
        .component
        .iter()
        .find(|c| c.name == name)
        .map(|c| resolve_dest(&c.dest, data_dir))
        .filter(|p| p.exists()) // avvia solo se il file è effettivamente presente
}

/// Termina eventuali istanze stantie identificate dal percorso del binario.
/// Usata prima di spawn_eullm: garantisce che solo la nostra istanza (con il
/// nostro modello) sia in ascolto. Best-effort: errori ignorati.
async fn kill_stale_process(bin: &Path) {
    let bin_str = bin.display().to_string();
    tracing::debug!("kill_stale_process: pkill -f {bin_str}");
    let _ = Command::new("pkill")
        .arg("-f")
        .arg(&bin_str)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    // Breve attesa affinché la porta venga liberata dal kernel
    tokio::time::sleep(Duration::from_millis(800)).await;
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
        tasks.push(tokio::spawn(async move {
            download_chunk(client, url, file, cs, ce, dl).await
        }));
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
