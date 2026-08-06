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
//!  4. Scarica/verifica ogni componente selezionato. Per eullm (solo se
//!     manage_subprocesses=true): controlla anche un override locale
//!     ({data}/bin/eullm.override.json, NON git-tracked — vedi sezione
//!     "eullm: controllo versione remota") e, ad ogni riavvio, se GitHub ha
//!     una release più recente — se sì e stdin è un terminale, chiede se
//!     scaricarla. Mai bloccante: rete irraggiungibile o avvio non
//!     interattivo (systemd/Docker) → skip silenzioso, si resta sulla
//!     versione già presente.
//!  5. Se manage_subprocesses=true: avvia qdrant + eullm (kill_on_drop).
//!  6. Attende /healthz (qdrant) e /api/tags (eullm).
//!  7. Ritorna ProcessGuard — al drop i figli ricevono SIGKILL.

use anyhow::{bail, Context, Result};
use futures_util::{stream, TryStreamExt};
use serde::Deserialize;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::process::Command;

use crate::config::{EullmSettings, Settings};

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
    /// Se presente: `url` punta a un archivio .tar.gz, questo è il path
    /// INTERNO del file da estrarre e scrivere in `dest`. sha256/size si
    /// riferiscono al file ESTRATTO (non all'archivio) — la verifica avviene
    /// dopo l'estrazione, così lo stamp-file fast-path (che hash-a `dest`)
    /// funziona invariato per entrambi i casi.
    #[serde(default)]
    archive_member: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    component: Vec<Component>,
}

fn load_manifest() -> Result<Manifest> {
    toml::from_str(MANIFEST_STR).context("manifest.toml non valido — bug di compilazione")
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn manifest_toml_parses_and_has_sane_fields() {
        let manifest = load_manifest().expect("manifest.toml deve fare parse");
        assert!(!manifest.component.is_empty());
        for comp in &manifest.component {
            assert_eq!(comp.sha256.len(), 64, "{}: sha256 non è a 64 caratteri hex", comp.name);
            assert!(comp.dest.contains("{data}"), "{}: dest senza placeholder {{data}}", comp.name);
            assert!(comp.url.starts_with("https://"), "{}: url non https", comp.name);
        }
        for name in ["tessdata-ita", "tessdata-eng"] {
            let comp = manifest
                .component
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("componente {name} mancante dal manifest"));
            assert!(comp.target.is_none(), "{name}: deve essere universale (nessun target)");
            assert!(comp.dest.ends_with(".traineddata"));
        }
    }
}

#[cfg(test)]
mod eullm_args_tests {
    use super::*;

    fn base_cfg() -> EullmSettings {
        EullmSettings {
            url: "http://localhost:11434".into(),
            model: "qwen3-14b".into(),
            num_ctx: 16384,
            num_predict: 4096,
            repeat_penalty: 1.3,
            keep_alive: -1,
            batch_size: 1,
            cache_type_k: None,
            cache_type_v: None,
            fit: false,
            unload_during_ingestion: false,
            model_override: None,
            n_cpu_moe: None,
        }
    }

    /// Un flag sbagliato qui impedisce a eullm di avviarsi in produzione —
    /// niente --fit (non esiste nella release pinnata v0.6.6, verificato dal
    /// binario), --ctx-size deve essere il TOTALE (num_ctx * batch_size).
    #[test]
    fn default_single_slot_no_cache_override() {
        let args = eullm_args(&base_cfg());
        assert_eq!(args, vec!["--cli", "--ctx-size", "16384", "--batch-size", "1"]);
    }

    #[test]
    fn ctx_size_is_num_ctx_times_batch_size() {
        let mut cfg = base_cfg();
        cfg.batch_size = 2;
        let args = eullm_args(&cfg);
        assert_eq!(args, vec!["--cli", "--ctx-size", "32768", "--batch-size", "2"]);
    }

    #[test]
    fn cache_type_flags_only_when_set() {
        let mut cfg = base_cfg();
        cfg.cache_type_k = Some("q8_0".into());
        cfg.cache_type_v = Some("q4_0".into());
        let args = eullm_args(&cfg);
        assert_eq!(
            args,
            vec![
                "--cli", "--ctx-size", "16384", "--batch-size", "1",
                "--cache-type-k", "q8_0", "--cache-type-v", "q4_0",
            ]
        );
    }

    #[test]
    fn no_fit_flag_unless_configured() {
        // --fit esiste solo da EuLLM-v0.6.9 in poi (verificato nel sorgente
        // eullm) — la pin di oggi per x86_64 è ancora v0.6.6, che clap
        // rifiuterebbe come flag sconosciuto. Di default (fit=false, il
        // default di EullmSettings) non deve mai comparire.
        for cfg in [base_cfg(), {
            let mut c = base_cfg();
            c.cache_type_k = Some("q8_0".into());
            c
        }] {
            assert!(!eullm_args(&cfg).contains(&"--fit".to_owned()));
        }
    }

    #[test]
    fn fit_flag_present_when_configured() {
        let mut cfg = base_cfg();
        cfg.fit = true;
        assert!(eullm_args(&cfg).contains(&"--fit".to_owned()));
    }

    #[test]
    fn n_cpu_moe_absent_by_default() {
        assert!(!eullm_args(&base_cfg()).contains(&"--n-cpu-moe".to_owned()));
    }

    #[test]
    fn n_cpu_moe_present_when_configured() {
        let mut cfg = base_cfg();
        cfg.n_cpu_moe = Some(24);
        let args = eullm_args(&cfg);
        let pos = args.iter().position(|a| a == "--n-cpu-moe").expect("--n-cpu-moe assente");
        assert_eq!(args[pos + 1], "24");
    }
}

#[cfg(test)]
mod archive_extraction_tests {
    use super::*;

    /// Verifica end-to-end reale (non solo che compili): estrae davvero
    /// lib/libpdfium.so da un archivio .tar.gz ufficiale pdfium-binaries e
    /// conferma che il file estratto combacia byte-per-byte con lo sha256
    /// pinnato in manifest.toml per il target linux-x86_64.
    ///
    /// Richiede (solo locale — vedi CLAUDE.md, niente CI):
    ///   PDFIUM_ARCHIVE_FOR_TEST=/path/a/pdfium-linux-x64.tgz
    ///   (scaricato da github.com/bblanchon/pdfium-binaries, tag chromium/7920)
    /// Skip gracioso se non impostata.
    #[test]
    fn extract_tar_gz_member_matches_manifest_pdfium_entry() {
        let Ok(archive) = std::env::var("PDFIUM_ARCHIVE_FOR_TEST") else {
            eprintln!("PDFIUM_ARCHIVE_FOR_TEST non impostata — skip (vedi doc del test)");
            return;
        };

        let out = std::env::temp_dir().join("i3k_pdfium_extract_test.so");
        let _ = std::fs::remove_file(&out);

        extract_tar_gz_member(Path::new(&archive), "lib/libpdfium.so", &out)
            .expect("estrazione deve riuscire");

        let got = sha256_file(&out).expect("sha256 del file estratto");
        let _ = std::fs::remove_file(&out);

        let manifest = load_manifest().expect("manifest.toml deve fare parse");
        let comp = manifest
            .component
            .iter()
            .find(|c| c.name == "pdfium" && c.target.as_deref() == Some("linux-x86_64"))
            .expect("componente pdfium linux-x86_64 mancante dal manifest");

        assert_eq!(
            got, comp.sha256,
            "il file estratto non combacia col sha256 pinnato in manifest.toml"
        );
        assert_eq!(comp.archive_member.as_deref(), Some("lib/libpdfium.so"));
    }
}

#[cfg(test)]
mod eullm_update_tests {
    use super::*;

    #[test]
    fn parse_semver_strips_known_prefixes() {
        assert_eq!(parse_semver("0.6.6"), Some((0, 6, 6)));
        assert_eq!(parse_semver("v0.6.6"), Some((0, 6, 6)));
        assert_eq!(parse_semver("EuLLM-v0.6.6"), Some((0, 6, 6)));
        assert_eq!(parse_semver("EuLLM-v1.12.103"), Some((1, 12, 103)));
    }

    #[test]
    fn parse_semver_rejects_unexpected_formats() {
        assert_eq!(parse_semver(""), None);
        assert_eq!(parse_semver("nightly"), None);
        assert_eq!(parse_semver("EuLLM-v0.6"), None);
        assert_eq!(parse_semver("EuLLM-v0.6.6-rc1"), None); // "6-rc1" non parsa come u32
    }

    /// La causa diretta del bug che questa feature avrebbe potuto introdurre
    /// se il confronto fosse stato per stringa invece che numerico: "0.6.9"
    /// vince su "0.6.10" lessicograficamente ('9' > '1'), ma non è la
    /// versione più recente. Il confronto DEVE essere sulla tupla numerica.
    #[test]
    fn version_tuple_compares_numerically_not_lexicographically() {
        let v9 = parse_semver("0.6.9").unwrap();
        let v10 = parse_semver("0.6.10").unwrap();
        assert!(v10 > v9, "0.6.10 deve essere considerata più recente di 0.6.9");
    }

    #[test]
    fn asset_hint_matches_every_pinned_eullm_target() {
        // Verifica OGNI componente eullm nel manifest, non solo il primo
        // trovato — debolezza del test precedente che NON ha intercettato il
        // bug reale (hint hardcoded su x86_64 mentre il manifest aveva già un
        // secondo target aarch64-cuda pinnato: su ARM64 scaricava l'asset
        // x86_64 sbagliato, "Exec format error" sull'Orion). Per ogni target
        // pinnato, eullm_asset_hint deve produrre un hint riconosciuto E
        // quell'hint deve combaciare col nome file dell'URL pinnato PER
        // QUEL target specifico.
        let manifest = load_manifest().expect("manifest.toml deve fare parse");
        let eullm_components: Vec<_> =
            manifest.component.iter().filter(|c| c.name == "eullm").collect();
        assert!(!eullm_components.is_empty(), "nessun componente eullm nel manifest");

        for comp in eullm_components {
            let asset_name = comp.url.rsplit('/').next().unwrap_or("");
            let hint = eullm_asset_hint(comp.target.as_deref()).unwrap_or_else(|| {
                panic!(
                    "eullm_asset_hint non riconosce il target {:?} (asset pinnato: {asset_name:?}) \
                     — estendila di pari passo quando si aggiunge un nuovo target eullm al manifest",
                    comp.target
                )
            });
            assert!(
                asset_name.contains(hint),
                "target {:?}: hint {hint:?} non combacia con l'asset pinnato {asset_name:?}",
                comp.target
            );
        }
    }
}

// ── Target detection (runtime) ────────────────────────────────────────────────

/// Target supportati, ordinati dal più specifico al più generico.
/// L'ordine è importante: `select_components` prende il PRIMO match per dest.
///
/// Schema target:
///   linux-x86_64-cuda   — Linux x86_64 con GPU NVIDIA (driver caricato)
///   linux-x86_64        — Linux x86_64 CPU-only / fallback
///   linux-aarch64, darwin-arm64, darwin-x86_64, windows-x86_64, windows-arm64
fn current_targets() -> Vec<&'static str> {
    let mut t = Vec::new();

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        // CUDA: /dev/nvidia0 esiste se il driver NVIDIA è caricato
        if std::path::Path::new("/dev/nvidia0").exists() {
            t.push("linux-x86_64-cuda");
        }
        t.push("linux-x86_64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        // Stessa euristica di x86_64 — vale anche su ARM64+dGPU NVIDIA via
        // PCIe (es. Radxa Orion O6 con GPU esterna), non solo Jetson/SoC.
        if std::path::Path::new("/dev/nvidia0").exists() {
            t.push("linux-aarch64-cuda");
        }
        t.push("linux-aarch64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    t.push("darwin-arm64");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    t.push("darwin-x86_64");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    t.push("windows-x86_64");
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    t.push("windows-arm64");

    t
}

/// Seleziona i componenti da scaricare/verificare.
///
/// Regola: per ogni `dest`, vince il componente col target PIÙ SPECIFICO
/// (primo in `targets`). I modelli (nessun target) vengono sempre inclusi.
/// Questo garantisce che su una macchina CUDA si scarichi la variante CUDA
/// e non anche quella CPU-only, anche se entrambe sono nel manifest.
fn select_components<'a>(manifest: &'a Manifest, targets: &[&str]) -> Vec<&'a Component> {
    let mut selected: Vec<&'a Component> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Binari: in ordine di priorità (più specifico prima)
    for &tgt in targets {
        for comp in &manifest.component {
            if comp.target.as_deref() == Some(tgt) && seen.insert(comp.dest.clone()) {
                selected.push(comp);
            }
        }
    }
    // 2. Modelli (nessun target — universali)
    for comp in &manifest.component {
        if comp.target.is_none() && seen.insert(comp.dest.clone()) {
            selected.push(comp);
        }
    }
    selected
}

// ── Risoluzione path ──────────────────────────────────────────────────────────

fn resolve_dest(dest: &str, data_dir: &Path) -> PathBuf {
    PathBuf::from(dest.replace("{data}", &data_dir.display().to_string()))
}

// ── Guardia processi supervisionati ───────────────────────────────────────────

/// Tiene in vita i processi figli. Al drop: SIGKILL (kill_on_drop=true).
pub struct ProcessGuard {
    _children: Vec<tokio::process::Child>,
    /// Se bootstrap ha avviato eullm, il path GGUF esatto usato per farlo.
    /// eullm accetta un path GGUF diretto nel campo "model" di /api/generate
    /// (vedi errore "Accepted formats: GGUF file path / ... / Registered
    /// name") — usando lo STESSO path con cui è stato avviato non serve
    /// nessuna registrazione (`eullm import-ollama`) né alcuno stato esterno
    /// che possa andare perso. None se manage_subprocesses=false (eullm
    /// esterno): in quel caso si usa Settings.eullm.model così com'è.
    pub eullm_model_path: Option<std::path::PathBuf>,
}

// ── Costanti ──────────────────────────────────────────────────────────────────

const DOWNLOAD_CONNECTIONS: usize = 8;

/// Dimensione di ciascun pezzo scaricato via Range request — indipendente
/// dal numero di connessioni concorrenti (vedi parallel_download). Pezzi
/// piccoli e numerosi invece di uno enorme per connessione: su un link a
/// banda irregolare (osservato con Starlink, dove il throughput per
/// connessione varia molto nel tempo) un pezzo sfortunato che rallenta
/// bloccava PRIMA l'intera connessione fino alla fine — con pezzi piccoli
/// un worker libero prende subito il pezzo successivo in coda invece di
/// restare fermo ad aspettare. Stesso principio di aria2c con -s più alto
/// di -x. Confermato con aria2c contro lo stesso file (qwen3-14b, stesso
/// URL del bootstrap): CN:8 reale, 16MiB/s — contro i ~3MB/s del nostro
/// codice con 8 pezzi fissi da total/8 l'uno. Non era un limite di
/// Starlink né del server: la granularità dei pezzi era il problema.
const PIECE_SIZE_BYTES: u64 = 16 * 1024 * 1024; // 16 MB

/// Numero di connessioni concorrenti per il download di ogni componente
/// (worker che processano la coda di pezzi da PIECE_SIZE_BYTES, vedi
/// sopra). Default 8. I3K_DOWNLOAD_CONNECTIONS permette di cambiarlo senza
/// ricompilare, per reti con caratteristiche molto diverse dal solito.
fn download_connections() -> usize {
    std::env::var("I3K_DOWNLOAD_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DOWNLOAD_CONNECTIONS)
}

const QDRANT_READY_TIMEOUT_SECS: u64 = 60;
const EULLM_READY_TIMEOUT_SECS: u64 = 600;
const POLL_INTERVAL_MS: u64 = 2_000;
const PROBE_TIMEOUT_SECS: u64 = 3;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Stato che attraversa le due fasi di boot — opaco a main.rs, serve solo a
/// non ripetere manifest/target detection tra provision_and_start_qdrant e
/// start_eullm.
pub struct Phase1 {
    manifest: Manifest,
    data_dir: std::path::PathBuf,
}

/// Fase 1: scarica/verifica tutti i componenti selezionati (eullm incluso —
/// solo il file, non lo avvia), poi avvia qdrant e attende /healthz. NON
/// avvia eullm: vedi start_eullm, chiamata separatamente così main.rs può
/// caricare l'embedding PRIMA quando settings.eullm.fit=true — --fit di
/// eullm legge la VRAM libera con cudaMemGetInfo al proprio avvio, quindi
/// deve vedere la VRAM già ridotta dall'embedding per offloadare i layer di
/// conseguenza (vedi audit Fase 1, punto 5a, e EullmSettings::fit).
pub async fn provision_and_start_qdrant(
    settings: &Settings,
) -> Result<(Phase1, Vec<tokio::process::Child>)> {
    let manifest = load_manifest()?;
    let data_dir = settings.data.data_path();
    let targets = current_targets();
    let selected = select_components(&manifest, &targets);

    // Struttura directory
    for subdir in &["bin", "models", "storage/qdrant", "db", "uploads", "backups"] {
        tokio::fs::create_dir_all(data_dir.join(subdir))
            .await
            .with_context(|| format!("mkdir {}/{subdir}", data_dir.display()))?;
    }

    // Pre-check spazio disco
    check_disk_space(&selected, &data_dir)?;

    // Scarica/verifica ogni componente selezionato. eullm ha un percorso
    // dedicato — solo quando lo gestiamo noi (manage_subprocesses=true, senza
    // sarebbe un binario che non arriviamo mai a spawnare): controlla anche un
    // eventuale override locale (versione più recente approvata a runtime, vedi
    // maybe_update_eullm) prima di ricadere sul pin del manifest.
    for comp in &selected {
        let dest = resolve_dest(&comp.dest, &data_dir);
        if comp.name == "eullm" && settings.data.manage_subprocesses {
            ensure_eullm_component(comp, &dest, &data_dir).await?;
        } else {
            ensure_component(comp, &dest).await?;
        }
    }

    let mut children: Vec<tokio::process::Child> = Vec::new();

    if settings.data.manage_subprocesses {
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
    } else {
        tracing::info!("manage_subprocesses=false — processi esterni attesi (qdrant)");
    }

    wait_for_url(
        &format!("{}/healthz", settings.qdrant.url),
        QDRANT_READY_TIMEOUT_SECS,
        "qdrant",
    )
    .await?;

    Ok((Phase1 { manifest, data_dir }, children))
}

/// Fase 2: avvia eullm (se manage_subprocesses=true) e attende /api/tags.
/// `children` sono i processi già avviati in fase 1 (qdrant) — combinati con
/// quello di eullm nel ProcessGuard finale, che tiene in vita ENTRAMBI.
pub async fn start_eullm(
    settings: &Settings,
    phase1: Phase1,
    mut children: Vec<tokio::process::Child>,
) -> Result<ProcessGuard> {
    let Phase1 { manifest, data_dir } = phase1;
    let mut eullm_model_path: Option<std::path::PathBuf> = None;

    if settings.data.manage_subprocesses {
        // eullm — decisione di avvio basata su presenza su disco, non sul target.
        // Il target filtra i DOWNLOAD (non scaricare CUDA binary su CPU);
        // ma se il file c'è, lo avviamo — il RAG dipende da eullm.
        //
        // EULLM__MODEL_OVERRIDE, se impostato, bypassa la ricerca del
        // componente "qwen3-14b" pinnato nel manifest (vedi EullmSettings::
        // model_override) — usato per puntare a un modello non pinnato,
        // es. un riferimento hf.co che eullm risolve/scarica da sé.
        let gguf = settings
            .eullm
            .model_override
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(|| find_by_name(&manifest, "qwen3-14b", &data_dir));
        match (find_by_name(&manifest, "eullm", &data_dir), gguf) {
            (Some(bin), Some(gguf)) => {
                kill_stale_process(&bin).await;
                children.push(spawn_eullm(&bin, &gguf, &settings.eullm)?);
                tracing::info!("eullm avviato: {} {}", bin.display(), gguf.display());
                eullm_model_path = Some(gguf);
            }
            _ => tracing::warn!(
                "eullm o modello (qwen3-14b / EULLM__MODEL_OVERRIDE) non trovati in {} — RAG senza LLM",
                data_dir.display()
            ),
        }
    } else {
        tracing::info!("manage_subprocesses=false — processi esterni attesi (eullm)");
    }

    wait_for_url(
        &format!("{}/api/tags", settings.eullm.url),
        EULLM_READY_TIMEOUT_SECS,
        "eullm",
    )
    .await?;

    Ok(ProcessGuard { _children: children, eullm_model_path })
}

/// Percorso classico (settings.eullm.fit=false, il default): qdrant ed eullm
/// avviati insieme, l'embedding carica dopo (vedi main.rs) — comportamento
/// INVARIATO rispetto a prima dello split in provision_and_start_qdrant +
/// start_eullm. Chi vuole l'ordine invertito (fit=true) chiama le due fasi
/// separatamente per intercalarci il caricamento dell'embedding.
pub async fn ensure_ready(settings: &Settings) -> Result<ProcessGuard> {
    let (phase1, children) = provision_and_start_qdrant(settings).await?;
    start_eullm(settings, phase1, children).await
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

    parallel_download(&comp.url, &partial, &comp.name, download_connections()).await?;

    match &comp.archive_member {
        Some(member) => extract_and_verify_member(comp, &partial, member, dest).await?,
        None => verify_and_place(comp, &partial, dest).await?,
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

/// Caso semplice (nessun archive_member): verifica sha256 del file scaricato
/// così com'è e lo sposta in `dest`.
async fn verify_and_place(comp: &Component, partial: &Path, dest: &Path) -> Result<()> {
    let expected = comp.sha256.clone();
    let p = partial.to_owned();
    let name = comp.name.clone();
    let got = tokio::task::spawn_blocking(move || {
        tracing::info!("{name}: verifica sha256 post-download…");
        sha256_file(&p)
    })
    .await
    .context("spawn_blocking sha256")?
    .context("sha256 calcolo")?;

    if got != expected {
        tokio::fs::remove_file(partial).await.ok();
        bail!(
            "{}: sha256 errato dopo download\n  atteso:  {}\n  trovato: {}",
            comp.name,
            expected,
            got
        );
    }
    tokio::fs::rename(partial, dest)
        .await
        .with_context(|| format!("rename {}", dest.display()))?;
    write_stamp(dest, &got).await;
    Ok(())
}

/// Caso archive_member: estrae `member` dall'archivio .tar.gz scaricato
/// (`partial`), verifica il sha256 del file ESTRATTO (comp.sha256 si
/// riferisce a quello, non all'archivio), lo sposta in `dest`, poi scarta
/// l'archivio. Così lo stamp-file fast-path resta invariato: hash-a sempre
/// `dest` e lo confronta con comp.sha256, comportamento identico per
/// componenti semplici ed estratti da archivio.
async fn extract_and_verify_member(
    comp: &Component,
    partial: &Path,
    member: &str,
    dest: &Path,
) -> Result<()> {
    let extracted = dest.with_extension("extracted");

    let archive_path = partial.to_owned();
    let member_owned = member.to_owned();
    let extracted_path = extracted.clone();
    let name = comp.name.clone();
    tokio::task::spawn_blocking(move || {
        tracing::info!("{name}: estrazione {member_owned} dall'archivio…");
        extract_tar_gz_member(&archive_path, &member_owned, &extracted_path)
    })
    .await
    .context("spawn_blocking estrazione")??;

    let expected = comp.sha256.clone();
    let ep = extracted.clone();
    let name2 = comp.name.clone();
    let got = tokio::task::spawn_blocking(move || {
        tracing::info!("{name2}: verifica sha256 del file estratto…");
        sha256_file(&ep)
    })
    .await
    .context("spawn_blocking sha256")?
    .context("sha256 calcolo")?;

    tokio::fs::remove_file(partial).await.ok(); // archivio non più necessario

    if got != expected {
        tokio::fs::remove_file(&extracted).await.ok();
        bail!(
            "{}: sha256 errato dopo estrazione\n  atteso:  {}\n  trovato: {}",
            comp.name,
            expected,
            got
        );
    }
    tokio::fs::rename(&extracted, dest)
        .await
        .with_context(|| format!("rename {}", dest.display()))?;
    write_stamp(dest, &got).await;
    Ok(())
}

/// Estrae un singolo file (`member`, path interno all'archivio) da un
/// .tar.gz e lo scrive in `out_path`. Sincrona/bloccante: va chiamata dentro
/// spawn_blocking.
fn extract_tar_gz_member(archive_path: &Path, member: &str, out_path: &Path) -> Result<()> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("apertura archivio {}", archive_path.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().context("lettura entries tar")? {
        let mut entry = entry.context("lettura entry tar")?;
        let path = entry.path().context("path entry tar")?.into_owned();
        if path.as_path() == Path::new(member) {
            let mut out = std::fs::File::create(out_path)
                .with_context(|| format!("creazione {}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out).context("estrazione file")?;
            return Ok(());
        }
    }
    bail!("membro '{member}' non trovato nell'archivio {}", archive_path.display())
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

// ── eullm: controllo versione remota + override locale ─────────────────────────
//
// eullm si auto-aggiorna spesso (fix indipendenti, vedi CLAUDE.md — è per questo
// che resta un processo separato e non viene compilato nel binario). Il pin in
// manifest.toml (version + sha256) resta la base di partenza affidabile e
// git-tracked, ma su richiesta esplicita dell'utente ad ogni riavvio si
// controlla se GitHub ha una release più recente e — SOLO se lo stdin è un
// terminale, per non bloccare mai un avvio non presidiato (systemd/Docker) —
// si chiede interattivamente se scaricarla.
//
// Una versione scaricata così non ha uno sha256 pre-verificato nel manifest:
// la fiducia è nell'approvazione esplicita dell'utente a quel momento, non in
// un hash pinnato in anticipo — è una differenza reale rispetto a tutti gli
// altri componenti di questo file, e va sempre loggata in chiaro.
//
// Persistenza: {data}/bin/eullm.override.json (locale, NON git-tracked) —
// ricorda la versione approvata (per non riscaricarla ad ogni riavvio) e
// l'ultima versione rifiutata (per non richiederla di nuovo finché non ne
// esce una ancora più recente).

const EULLM_RELEASES_API: &str = "https://api.github.com/repos/eullm/eullm/releases/latest";
const EULLM_UPDATE_CHECK_TIMEOUT_SECS: u64 = 10;

/// Sottostringa che identifica l'asset eullm per la nostra piattaforma nel nome
/// file della release GitHub (es. "eullm-linux-x64-cuda-12.8" → hint
/// "linux-x64-cuda"). Derivata dal `target` del Component GIÀ selezionato per
/// questa macchina (current_targets() → select_components(), vedi
/// provision_and_start_qdrant) — non una euristica separata da tenere
/// sincronizzata a mano.
///
/// Bug reale che questa funzione sostituisce: prima era una const hardcoded
/// a "linux-x64-cuda" indipendentemente dalla piattaforma — su ARM64 (Radxa
/// Orion O6) il controllo aggiornamenti trovava comunque l'asset x86_64 e lo
/// scaricava, causando "Exec format error" all'avvio di eullm. None = target
/// senza asset eullm noto → skip del controllo (mai un default silenzioso
/// sbagliato).
fn eullm_asset_hint(target: Option<&str>) -> Option<&'static str> {
    match target {
        Some("linux-x86_64-cuda") => Some("linux-x64-cuda"),
        Some("linux-aarch64-cuda") => Some("linux-arm64-cuda"),
        Some("linux-aarch64") => Some("linux-arm64"),
        _ => None,
    }
}

#[derive(Debug, serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, serde::Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct EullmOverride {
    installed_version: Option<String>,
    installed_sha256: Option<String>,
    installed_url: Option<String>,
    declined_version: Option<String>,
}

fn eullm_override_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join("eullm.override.json")
}

async fn load_eullm_override(data_dir: &Path) -> EullmOverride {
    match tokio::fs::read_to_string(eullm_override_path(data_dir)).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => EullmOverride::default(),
    }
}

async fn save_eullm_override(data_dir: &Path, ov: &EullmOverride) {
    if let Ok(s) = serde_json::to_string_pretty(ov) {
        let _ = tokio::fs::write(eullm_override_path(data_dir), s).await;
    }
}

/// "0.6.6" / "v0.6.6" / "EuLLM-v0.6.6" → (0,6,6). None se il formato non torna
/// (es. GitHub cambia schema di tag) — trattato come "skip check", mai come
/// errore fatale.
fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.strip_prefix("EuLLM-v").or_else(|| s.strip_prefix('v')).unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn stdin_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

async fn fetch_latest_eullm_release() -> Result<GhRelease> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(EULLM_UPDATE_CHECK_TIMEOUT_SECS))
        .build()?;
    let resp = client
        .get(EULLM_RELEASES_API)
        .header("User-Agent", "i3k-rag-engine")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("richiesta GitHub releases eullm")?;
    if !resp.status().is_success() {
        bail!("GitHub API {} — {}", resp.status(), EULLM_RELEASES_API);
    }
    resp.json::<GhRelease>().await.context("parse risposta GitHub releases")
}

/// Sceglie quale Component usare per provisionare eullm: l'override locale
/// se presente e ancora più recente del pin del manifest, altrimenti il pin.
/// Se il pin ha nel frattempo raggiunto/superato l'override (es. abbiamo
/// aggiornato manifest.toml), l'override è considerato superato e scartato.
async fn effective_eullm_component(pinned: &Component, data_dir: &Path) -> Component {
    let mut ov = load_eullm_override(data_dir).await;

    let override_is_stale = match (&ov.installed_version, pinned.version.as_deref()) {
        (Some(ov_v), Some(pin_v)) => {
            matches!((parse_semver(ov_v), parse_semver(pin_v)), (Some(a), Some(b)) if a <= b)
        }
        _ => false,
    };
    if override_is_stale {
        tracing::info!(
            "eullm: il pin del manifest ha raggiunto/superato l'override locale, torno al pin"
        );
        ov = EullmOverride::default();
        save_eullm_override(data_dir, &ov).await;
    }

    match (&ov.installed_version, &ov.installed_sha256, &ov.installed_url) {
        (Some(v), Some(sha), Some(url)) => Component {
            version: Some(v.clone()),
            sha256: sha.clone(),
            url: url.clone(),
            ..pinned.clone()
        },
        _ => pinned.clone(),
    }
}

/// Provisiona eullm: usa l'override locale se valido, altrimenti il pin del
/// manifest (comportamento identico a ensure_component per ogni altro
/// componente) — poi controlla se esiste una versione ancora più recente.
async fn ensure_eullm_component(pinned: &Component, dest: &Path, data_dir: &Path) -> Result<()> {
    let effective = effective_eullm_component(pinned, data_dir).await;
    ensure_component(&effective, dest).await?;
    maybe_update_eullm(pinned, dest, data_dir).await;
    Ok(())
}

/// Controlla se GitHub ha una release eullm più recente di quella attualmente
/// effettiva (override locale se presente, altrimenti il pin) e, se sì,
/// chiede interattivamente — SOLO se stdin è un terminale — se scaricarla.
/// Non fatale in nessun caso: rete irraggiungibile, parsing fallito o nessun
/// asset compatibile vengono solo loggati; l'avvio prosegue sempre con la
/// versione già presente.
async fn maybe_update_eullm(pinned: &Component, dest: &Path, data_dir: &Path) {
    let mut ov = load_eullm_override(data_dir).await;
    let current_version =
        ov.installed_version.clone().or_else(|| pinned.version.clone()).unwrap_or_default();

    let release = match fetch_latest_eullm_release().await {
        Ok(r) => r,
        Err(e) => {
            tracing::info!(error = ?e, "controllo versione eullm saltato (GitHub non raggiungibile)");
            return;
        }
    };

    let (Some(latest), Some(current)) =
        (parse_semver(&release.tag_name), parse_semver(&current_version))
    else {
        tracing::warn!(tag = %release.tag_name, installed = %current_version, "formato versione eullm inatteso, skip controllo aggiornamenti");
        return;
    };
    if latest <= current {
        tracing::info!(installed = %current_version, "eullm già alla versione più recente disponibile");
        return;
    }
    let latest_str = format!("{}.{}.{}", latest.0, latest.1, latest.2);

    if ov.declined_version.as_deref() == Some(latest_str.as_str()) {
        tracing::info!(latest = %latest_str, "nuova versione eullm già rifiutata in precedenza, non richiedo di nuovo");
        return;
    }

    let Some(hint) = eullm_asset_hint(pinned.target.as_deref()) else {
        tracing::warn!(target = ?pinned.target, "eullm: piattaforma senza asset di aggiornamento noto, skip controllo versione");
        return;
    };
    let Some(asset) = release.assets.iter().find(|a| a.name.contains(hint)) else {
        tracing::warn!(latest = %latest_str, hint = %hint, "nuova versione eullm trovata ma nessun asset per questa piattaforma, skip");
        return;
    };

    tracing::warn!(
        installed = %current_version,
        latest = %latest_str,
        release_notes = %release.html_url,
        "eullm: nuova versione disponibile"
    );

    if !stdin_is_tty() {
        tracing::warn!(
            "avvio non interattivo (nessun terminale su stdin) — non chiedo conferma, continuo \
             con la versione {current_version}. Per aggiornare: avvia da un terminale, oppure \
             elimina {} per far ripartire il controllo alla prossima occasione interattiva.",
            eullm_override_path(data_dir).display()
        );
        return;
    }

    eprintln!();
    eprintln!("  eullm: nuova versione disponibile — installata: {current_version}, ultima: {latest_str}");
    eprintln!("  Note di rilascio: {}", release.html_url);
    eprint!("  Scaricare e usare la nuova versione ora? [s/N]: ");
    {
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        tracing::warn!("lettura risposta da stdin fallita, continuo con la versione corrente");
        return;
    }
    let yes = matches!(answer.trim().to_lowercase().as_str(), "s" | "si" | "sì" | "y" | "yes");

    if !yes {
        tracing::info!(latest = %latest_str, "aggiornamento eullm rifiutato, resto sulla versione {current_version}");
        ov.declined_version = Some(latest_str);
        save_eullm_override(data_dir, &ov).await;
        return;
    }

    tracing::info!(url = %asset.browser_download_url, size = asset.size, "download eullm {latest_str}…");
    let partial = dest.with_extension("partial");
    let _ = tokio::fs::remove_file(&partial).await;

    if let Err(e) = parallel_download(
        &asset.browser_download_url,
        &partial,
        &format!("eullm {latest_str}"),
        download_connections(),
    )
    .await
    {
        tracing::warn!(error = ?e, "download eullm {latest_str} fallito, resto sulla versione corrente");
        let _ = tokio::fs::remove_file(&partial).await;
        return;
    }

    let hash_target = partial.clone();
    let sha256 = match tokio::task::spawn_blocking(move || sha256_file(&hash_target)).await {
        Ok(Ok(h)) => h,
        _ => {
            tracing::warn!("sha256 post-download fallito, scarto il file scaricato");
            let _ = tokio::fs::remove_file(&partial).await;
            return;
        }
    };

    if let Err(e) = tokio::fs::rename(&partial, dest).await {
        tracing::warn!(error = ?e, "impossibile installare eullm {latest_str}");
        return;
    }
    if pinned.exec {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755)).await;
    }
    write_stamp(dest, &sha256).await;

    tracing::warn!(
        version = %latest_str,
        sha256 = %sha256,
        "eullm aggiornato — ATTENZIONE: a differenza degli altri componenti, questa versione NON \
         aveva uno sha256 pre-pinnato nel manifest: lo sha256 sopra è quello effettivamente \
         scaricato ora, approvato da te a runtime, non verificato in anticipo. Annotalo se vuoi \
         pinnarlo in manifest.toml in un prossimo aggiornamento del repo."
    );

    ov.installed_version = Some(latest_str);
    ov.installed_sha256 = Some(sha256);
    ov.installed_url = Some(asset.browser_download_url.clone());
    ov.declined_version = None;
    save_eullm_override(data_dir, &ov).await;
}

// ── Spazio disco ──────────────────────────────────────────────────────────────

fn check_disk_space(selected: &[&Component], data_dir: &Path) -> Result<()> {
    struct Item<'a> {
        label: &'a str,
        size: u64,
    }

    let mut needed: Vec<Item> = Vec::new();
    for comp in selected {
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

/// Cerca per nome con priorità target (usato per decidere cosa avviare come qdrant).
/// Itera i target dal più specifico: prende il primo match.
fn find_component(manifest: &Manifest, name: &str, data_dir: &Path, targets: &[&str]) -> Option<PathBuf> {
    for &tgt in targets {
        if let Some(comp) = manifest.component.iter()
            .find(|c| c.name == name && c.target.as_deref() == Some(tgt))
        {
            return Some(resolve_dest(&comp.dest, data_dir));
        }
    }
    None
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

/// --ctx-size è il TOTALE eullm (llama.cpp-style), diviso tra i `batch_size`
/// slot concorrenti — quindi num_ctx (contesto per connessione, quello che
/// conta per far stare un prompt RAG) va moltiplicato per batch_size qui.
/// --fit NON esiste nella release pinnata (v0.6.6, verificato dal binario:
/// nessun auto-sizing) — senza --ctx-size/--batch-size espliciti eullm parte
/// al suo default (4096 ctx, 1 slot), troppo piccolo per un prompt RAG con
/// più di un paio di documenti indicizzati (causa nota di risposte vuote o
/// troncate a metà frase).
/// Pura e testabile: costruisce gli argomenti CLI (senza il model_path, che
/// è un Path e complicherebbe i confronti nei test — aggiunto separatamente).
fn eullm_args(cfg: &EullmSettings) -> Vec<String> {
    let ctx_size_total = cfg.num_ctx * cfg.batch_size;
    let mut args = vec![
        "--cli".to_owned(),
        "--ctx-size".to_owned(),
        ctx_size_total.to_string(),
        "--batch-size".to_owned(),
        cfg.batch_size.to_string(),
    ];
    if let Some(kt) = &cfg.cache_type_k {
        args.push("--cache-type-k".to_owned());
        args.push(kt.clone());
    }
    if let Some(vt) = &cfg.cache_type_v {
        args.push("--cache-type-v".to_owned());
        args.push(vt.clone());
    }
    if let Some(n) = cfg.n_cpu_moe {
        args.push("--n-cpu-moe".to_owned());
        args.push(n.to_string());
    }
    if cfg.fit {
        args.push("--fit".to_owned());
    }
    args
}

fn spawn_eullm(bin: &Path, model_path: &Path, cfg: &EullmSettings) -> Result<tokio::process::Child> {
    let mut cmd = Command::new(bin);
    cmd.arg("run").arg(model_path).args(eullm_args(cfg));

    cmd.kill_on_drop(true)
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

#[derive(Debug, Deserialize)]
struct EullmTagsResponse {
    #[serde(default)]
    models: Vec<EullmTagEntry>,
}

#[derive(Debug, Deserialize)]
struct EullmTagEntry {
    name: String,
}

/// Interroga /api/tags e ritorna il `name` del PRIMO modello in lista — è
/// quello attivo (eullm non implementa /api/ps: "first model in the list
/// is the active one", stesso comportamento documentato per rag-
/// enterprise-pro). Serve perché per un riferimento hf.co (a differenza di
/// un path GGUF locale) eullm normalizza il nome in un formato canonico
/// DIVERSO da quello con cui è stato lanciato — osservato in produzione:
/// lanciato con "hf.co/bartowski/Qwen_Qwen3.6-35B-A3B-GGUF:Q4_K_M",
/// /api/tags lo riporta come "qwen_qwen3.6-35b-a3b-gguf-q4_k_m", e SOLO
/// quel nome canonico è accettato nel campo "model" di /api/generate — il
/// riferimento hf.co originale dà 500 "Model ... not found" (i due punti
/// vengono mangiati da una normalizzazione lato eullm pensata per i tag
/// stile Ollama, che rompe la sintassi repo:quant di hf.co). None se la
/// query fallisce o la lista è vuota: il chiamante ricade sul nome di
/// lancio (vedi build_eullm_client in main.rs).
pub async fn fetch_active_model_name(eullm_url: &str) -> Option<String> {
    let resp = reqwest::Client::new()
        .get(format!("{eullm_url}/api/tags"))
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        .ok()?;
    let body: EullmTagsResponse = resp.json().await.ok()?;
    body.models.into_iter().next().map(|m| m.name)
}

#[cfg(test)]
mod eullm_tags_tests {
    use super::*;

    #[test]
    fn parses_real_tags_response_first_model_is_active() {
        // Fixture: risposta reale di /api/tags su Orion con Qwen3.6-35B-A3B
        // caricato via riferimento hf.co (eullm v0.6.11) — il PRIMO modello
        // in lista è quello attivo.
        let json = r#"{"models":[
            {"details":{"family":"","format":"gguf","parameter_size":"","quantization_level":"Q4_K_M"},"digest":"","name":"qwen_qwen3.6-35b-a3b-gguf-q4_k_m","size":0},
            {"details":{"display_name":"Qwen3 0.6B Instruct","domain":"general","family":"qwen3","format":"gguf","parameter_size":"0.6B","quantization_level":"Q4_K_M","source_model":"unsloth/Qwen3-0.6B-GGUF"},"digest":"","name":"qwen-0.6b","size":500000000}
        ]}"#;
        let parsed: EullmTagsResponse = serde_json::from_str(json).expect("deve fare parse");
        assert_eq!(parsed.models[0].name, "qwen_qwen3.6-35b-a3b-gguf-q4_k_m");
    }

    #[test]
    fn empty_models_list_parses_ok() {
        let parsed: EullmTagsResponse = serde_json::from_str(r#"{"models":[]}"#).unwrap();
        assert!(parsed.models.is_empty());
    }
}

// ── Download parallelo multi-chunk ────────────────────────────────────────────

async fn parallel_download(url: &str, dest: &Path, display_name: &str, n: usize) -> Result<()> {
    // http1_only: se il server negozia HTTP/2, reqwest multiplexerebbe le N
    // richieste Range "concorrenti" sulla STESSA connessione TCP — niente
    // aggregazione di banda reale, tutte condividono un'unica finestra di
    // congestione, sostanzialmente la velocità di UNA connessione (esatto
    // sintomo osservato: il nostro codice non supera mai ~3MB/s mentre
    // aria2c, HTTP/1.1 con connessioni realmente separate per pezzo, sullo
    // stesso file/server sostiene 15-16MB/s). Con HTTP/1.1 ogni richiesta
    // concorrente apre la propria connessione, aggregando banda per davvero.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3600))
        .http1_only()
        .build()?;

    // Probe con Range: bytes=0-0 (1 byte) invece di una GET piena: se il
    // server risponde 206 il supporto Range è verificato per davvero (non
    // solo dichiarato in accept-ranges, che alcuni server/proxy annunciano
    // senza onorarlo — vedi il controllo 206 in fetch_chunk_once) e la size
    // totale si legge da Content-Range: bytes 0-0/{total} nella stessa
    // risposta, senza una richiesta separata.
    let probe = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .context("probe GET")?;
    let probe_status = probe.status();

    if probe_status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // Range non valido su questo file (tipicamente 0 byte): richiesta
        // pulita senza Range invece di trattarlo come errore fatale.
        drop(probe);
        let full = client.get(url).send().await.context("GET (fallback 416)")?;
        if !full.status().is_success() {
            bail!("HTTP {} — {display_name} ({url})", full.status());
        }
        return download_streaming(full, dest, display_name).await;
    }
    if !probe_status.is_success() && probe_status != reqwest::StatusCode::PARTIAL_CONTENT {
        bail!("HTTP {probe_status} — {display_name} ({url})");
    }
    let final_url = probe.url().to_string();
    let accepts_ranges = probe_status == reqwest::StatusCode::PARTIAL_CONTENT;

    let total: u64 = if accepts_ranges {
        probe
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    } else {
        probe
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };

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

    let pieces: Vec<(u64, u64)> = {
        let mut v = Vec::new();
        let mut s = 0u64;
        while s < total {
            let e = (s + PIECE_SIZE_BYTES - 1).min(total - 1);
            v.push((s, e));
            s += PIECE_SIZE_BYTES;
        }
        v
    };

    use std::io::IsTerminal;
    let is_tty = std::io::stderr().is_terminal();
    let dl_ref = Arc::clone(&downloaded);
    let name_str = display_name.to_owned();
    let progress_task = tokio::spawn(async move {
        let mut last_done = 0u64;
        let mut last_tick = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let done = dl_ref.load(Ordering::Relaxed);
            if done == 0 {
                continue;
            }
            let pct = done as f64 / total as f64 * 100.0;
            // Velocità nella finestra recente (~3s), non la media cumulativa
            // dall'inizio: quella risponde troppo lentamente a rallentamenti
            // o riprese reali (es. micro-stalli da handoff satellitari su
            // Starlink) — un numero basso all'inizio del download resta
            // "congelato" per minuti anche se la velocità reale è già
            // tornata alta, mostrando un dato fuorviante.
            let now = Instant::now();
            let window = now.duration_since(last_tick).as_secs_f64().max(0.001);
            let rate = done.saturating_sub(last_done) as f64 / window;
            last_done = done;
            last_tick = now;
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

    // n worker concorrenti pescano dalla coda di pezzi: un pezzo sfortunato
    // che rallenta non blocca un intero worker fino alla fine come con un
    // range enorme fisso — appena un worker libera lo slot, prende il
    // pezzo successivo in coda (vedi commento su PIECE_SIZE_BYTES).
    stream::iter(pieces.into_iter().map(Ok::<(u64, u64), anyhow::Error>))
        .try_for_each_concurrent(n, |(cs, ce)| {
            let client = client.clone();
            let url = final_url.clone();
            let file = Arc::clone(&file);
            let dl = Arc::clone(&downloaded);
            async move { download_chunk(client, url, file, cs, ce, dl).await }
        })
        .await
        .context("chunk download")?;

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

const CHUNK_MAX_ATTEMPTS: u32 = 5;
const CHUNK_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Un pezzo (max PIECE_SIZE_BYTES) può capitare su una connessione che un
/// micro-drop da handover satellitare (~ogni 15s su Starlink) degrada a
/// metà scaricamento — senza retry, quella connessione resta lenta finché
/// TCP non rifà da sola la slow-start. Qui invece: fino a 5 tentativi,
/// ciascuno con un timeout di 120s, backoff 1s/2s/4s/8s tra un tentativo e
/// l'altro — un tentativo scaduto/fallito si abbandona e il successivo
/// riparte su una connessione NUOVA invece di aspettare che quella vecchia
/// si riprenda. Stessi parametri usati da eullm per lo stesso problema.
async fn download_chunk(
    client: reqwest::Client,
    url: String,
    file: Arc<std::fs::File>,
    start: u64,
    end: u64,
    downloaded: Arc<AtomicU64>,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=CHUNK_MAX_ATTEMPTS {
        match tokio::time::timeout(CHUNK_ATTEMPT_TIMEOUT, fetch_chunk_once(&client, &url, start, end, &downloaded)).await {
            Ok(Ok(buf)) => {
                // I byte sono già stati aggiunti a downloaded in streaming da
                // fetch_chunk_once (ProgressGuard, committato a tentativo
                // riuscito) — qui resta solo la scrittura su disco.
                let f = Arc::clone(&file);
                tokio::task::spawn_blocking(move || f.write_all_at(&buf, start))
                    .await
                    .context("spawn_blocking write")?
                    .context("pwrite")?;
                return Ok(());
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    attempt, max_attempts = CHUNK_MAX_ATTEMPTS, error = %e,
                    range = format!("bytes={start}-{end}"),
                    "pezzo fallito, riprovo su connessione nuova"
                );
                last_err = Some(e);
            }
            Err(_elapsed) => {
                tracing::warn!(
                    attempt, max_attempts = CHUNK_MAX_ATTEMPTS,
                    range = format!("bytes={start}-{end}"),
                    timeout_s = CHUNK_ATTEMPT_TIMEOUT.as_secs(),
                    "pezzo troppo lento (timeout), riprovo su connessione nuova"
                );
                last_err = Some(anyhow::anyhow!("timeout dopo {}s", CHUNK_ATTEMPT_TIMEOUT.as_secs()));
            }
        }
        if attempt < CHUNK_MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await; // 1s,2s,4s,8s
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("download pezzo fallito dopo {CHUNK_MAX_ATTEMPTS} tentativi")))
}

/// Contabilizza i byte di un tentativo su `downloaded` MAN MANO che
/// arrivano dalla rete, non in un solo scatto da PIECE_SIZE_BYTES a fine
/// pezzo — è quello scatto in blocco il motivo per cui il rate mostrato a
/// video "oscillava" (5/11/16/21/27/32/37 MB/s): con tick del progress_task
/// ogni 3s e un pezzo da 16MB completato ogni ~8s per worker, il contatore
/// restava fermo per tick interi e poi saltava di N*16MB tutti insieme —
/// N=1,2,3,4,5,6,7 pezzi completati nello stesso tick spiega ESATTAMENTE
/// 5.3/10.7/16.0/21.3/26.7/32.0/37.3 MB/s, i valori osservati (vedi log
/// reale del 2026-07-10). La velocità media era già corretta (~15MB/s), era
/// solo la lettura istantanea a essere quantizzata.
///
/// Se il tentativo fallisce o viene abbandonato per timeout (drop della
/// future da parte di tokio::time::timeout in download_chunk — nessun punto
/// di ritorno esplicito da cui richiamare un rollback esplicito), il Drop di
/// questa guardia sottrae esattamente quanto aggiunto da QUESTO tentativo:
/// il contatore resta corretto anche quando un pezzo va ritentato, senza
/// doppio conteggio. commit() disarma il rollback a tentativo riuscito.
struct ProgressGuard<'a> {
    counter: &'a AtomicU64,
    added: u64,
}

impl<'a> ProgressGuard<'a> {
    fn new(counter: &'a AtomicU64) -> Self {
        Self { counter, added: 0 }
    }

    fn add(&mut self, n: u64) {
        self.counter.fetch_add(n, Ordering::Relaxed);
        self.added += n;
    }

    fn commit(mut self) {
        self.added = 0;
    }
}

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        if self.added > 0 {
            self.counter.fetch_sub(self.added, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod progress_guard_tests {
    use super::*;

    #[test]
    fn commit_leaves_counter_incremented() {
        let counter = AtomicU64::new(0);
        let mut guard = ProgressGuard::new(&counter);
        guard.add(100);
        guard.add(50);
        guard.commit();
        assert_eq!(counter.load(Ordering::Relaxed), 150);
    }

    #[test]
    fn drop_without_commit_rolls_back_only_this_attempt() {
        let counter = AtomicU64::new(1_000); // byte già committati da altri pezzi
        {
            let mut guard = ProgressGuard::new(&counter);
            guard.add(100);
            guard.add(50);
            assert_eq!(counter.load(Ordering::Relaxed), 1_150);
            // guard esce dallo scope senza commit() — es. errore o timeout
        }
        assert_eq!(counter.load(Ordering::Relaxed), 1_000);
    }
}

/// Un solo tentativo: scarica il pezzo [start,end] in un buffer in RAM (max
/// PIECE_SIZE_BYTES), contabilizzando i byte su `downloaded` in streaming
/// via ProgressGuard man mano che arrivano (vedi sopra). La scrittura su
/// disco resta responsabilità del chiamante, una volta sola, solo a
/// tentativo riuscito (vedi download_chunk).
async fn fetch_chunk_once(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    downloaded: &AtomicU64,
) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .header("Range", format!("bytes={start}-{end}"))
        .send()
        .await
        .context("range GET")?;

    // Se il server ignora la Range e risponde 200 invece di 206, questo
    // "pezzo" riceve l'INTERO file invece del solo pezzo richiesto — con n
    // worker paralleli vorrebbe dire scaricare il file n volte, gran parte
    // dei byte scartati/sovrascritti: banda sprecata, non guadagnata.
    // Verificato solo qui (non nella probe) perché accept-ranges dichiarato
    // può non essere onorato davvero dal server/proxy per ogni richiesta.
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        tracing::warn!(
            status = %resp.status(),
            range = format!("bytes={start}-{end}"),
            url = %url,
            "server non ha risposto 206 Partial Content a una richiesta Range — \
             probabile download ridondante dell'intero file invece del solo pezzo"
        );
    }

    let mut resp = resp;
    let expected = (end - start + 1) as usize;
    let mut buf: Vec<u8> = Vec::with_capacity(expected);
    let mut guard = ProgressGuard::new(downloaded);
    while let Some(chunk) = resp.chunk().await.context("chunk read")? {
        guard.add(chunk.len() as u64);
        buf.extend_from_slice(&chunk);
    }
    guard.commit();
    Ok(buf)
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
