//! Embedding service: BAAI/bge-m3 via Candle (in-process, GPU/CPU).
//!
//! Parità con Python (embeddings_service.py):
//! - SentenceTransformer("BAAI/bge-m3", normalize_embeddings=True)
//! - Architettura: XLM-RoBERTa → weight keys prefissati con "roberta"
//! - Pooling: CLS (indice 0 della sequenza)
//! - L2-normalizzazione su query E documenti
//! - GPU batch=4 (conservativo: 14b + bge-m3 ~11 GB su 16 GB), CPU batch=2
//! - OOM CUDA → fallback CPU automatico sul batch
//!
//! Device selection (vedi load()): la CPU può essere raggiunta per due strade
//! ben distinte, e non vanno confuse — DeviceStatus le tiene separate:
//!   - CpuByConfig: build senza feature "cuda", CPU è la scelta attesa.
//!   - CpuFallback: CUDA richiesta ma fallita dopo i retry — degrado reale,
//!     va sempre segnalato forte (log error! + esposto via GET /info).

use anyhow::{bail, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokenizers::Tokenizer;

#[allow(dead_code)]
pub const EMBED_DIM: usize = 1024;
const GPU_BATCH: usize = 4;
const CPU_BATCH: usize = 2;
/// Tentativi di init CUDA (device + caricamento pesi) prima di rassegnarsi alla CPU.
const CUDA_LOAD_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// CUDA richiesta (feature "cuda" compilata) e riuscita.
    Gpu,
    /// CPU per scelta di build (feature "cuda" non compilata) — non è un fallback.
    CpuByConfig,
    /// CUDA richiesta ma fallita dopo CUDA_LOAD_ATTEMPTS tentativi — degrado reale.
    CpuFallback,
    /// CPU deliberatamente, tra una finestra di ingestione e l'altra
    /// (EmbeddingsSettings::swap_during_ingestion=true) — non è un degrado:
    /// la VRAM è riservata a eullm fuori dall'ingestione per design.
    CpuParked,
}

pub struct EmbeddingService {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    device_status: DeviceStatus,
    model_id: String,
}

impl EmbeddingService {
    /// Carica bge-m3. Se la feature "cuda" è compilata, tenta CUDA per
    /// CUDA_LOAD_ATTEMPTS tentativi (backoff 1s poi 2s tra un tentativo e
    /// l'altro) prima di ricadere su CPU. `require_gpu=true`
    /// (EMBEDDINGS__REQUIRE_GPU) fa fallire l'avvio invece di degradare in
    /// silenzio — un'ingestione a 17 minuti non deve mai passare inosservata.
    pub fn load(model_id: &str, require_gpu: bool) -> Result<Self> {
        if !cfg!(feature = "cuda") {
            if require_gpu {
                bail!(
                    "EMBEDDINGS__REQUIRE_GPU=true ma il binario è compilato senza \
                     --features cuda: nessun supporto GPU possibile per l'embedding. \
                     Ricompila con --features cuda (o ocr,cuda)."
                );
            }
            tracing::info!(model_id, "embedding su CPU (build senza feature \"cuda\")");
            let mut svc = Self::load_on(model_id, &Device::Cpu)?;
            svc.device_status = DeviceStatus::CpuByConfig;
            return Ok(svc);
        }

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 1..=CUDA_LOAD_ATTEMPTS {
            let outcome = Self::try_cuda_device(0).and_then(|dev| Self::load_on(model_id, &dev));
            match outcome {
                Ok(mut svc) => {
                    svc.device_status = DeviceStatus::Gpu;
                    tracing::info!(model_id, attempt, "embedding model pronto su GPU (CUDA)");
                    return Ok(svc);
                }
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = CUDA_LOAD_ATTEMPTS,
                        error = ?e,
                        "init CUDA per l'embedding fallito (tentativo {attempt}/{CUDA_LOAD_ATTEMPTS})"
                    );
                    last_err = Some(e);
                    if attempt < CUDA_LOAD_ATTEMPTS {
                        std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                    }
                }
            }
        }
        let e = last_err.expect("almeno un tentativo eseguito nel loop sopra");

        if require_gpu {
            return Err(e.context(
                "EMBEDDINGS__REQUIRE_GPU=true: CUDA ha fallito dopo tutti i tentativi — \
                 avvio interrotto invece di degradare su CPU",
            ));
        }

        tracing::error!(
            error = ?e,
            "\n================================================================\n\
             EMBEDDING IN FALLBACK SU CPU — CUDA fallita dopo {CUDA_LOAD_ATTEMPTS} tentativi.\n\
             L'ingestione documenti sarà MOLTO più lenta (minuti anziché secondi).\n\
             Causa reale nel campo 'error' qui sopra (chain completa).\n\
             Imposta EMBEDDINGS__REQUIRE_GPU=true per fallire subito invece di degradare.\n\
             ================================================================"
        );
        let mut svc = Self::load_on(model_id, &Device::Cpu)
            .context("fallback CPU dopo fallimento CUDA")?;
        svc.device_status = DeviceStatus::CpuFallback;
        Ok(svc)
    }

    #[cfg(feature = "cuda")]
    fn try_cuda_device(ordinal: usize) -> Result<Device> {
        Device::new_cuda(ordinal).context("Device::new_cuda")
    }
    #[cfg(not(feature = "cuda"))]
    fn try_cuda_device(_ordinal: usize) -> Result<Device> {
        unreachable!("chiamato solo quando la feature \"cuda\" è attiva")
    }

    pub fn device_status(&self) -> DeviceStatus {
        self.device_status
    }

    pub fn device_label(&self) -> &'static str {
        match self.device_status {
            DeviceStatus::Gpu => "gpu",
            DeviceStatus::CpuByConfig => "cpu (build senza GPU)",
            DeviceStatus::CpuFallback => "cpu (FALLBACK: CUDA fallita all'avvio, vedi log)",
            DeviceStatus::CpuParked => "cpu (in attesa — VRAM riservata a eullm fuori ingestione)",
        }
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Carica bge-m3 su CPU con status CpuParked — usato sia all'avvio
    /// quando EMBEDDINGS__SWAP_DURING_INGESTION=true (la VRAM resta libera
    /// per eullm finché non parte un'ingestione) sia per il rientro su CPU
    /// a fine ingestione (vedi AppState::swap_embeddings_to_cpu).
    pub fn load_cpu_parked(model_id: &str) -> Result<Self> {
        let mut svc = Self::load_on(model_id, &Device::Cpu)?;
        svc.device_status = DeviceStatus::CpuParked;
        Ok(svc)
    }

    /// Carica bge-m3 su GPU per la finestra di ingestione, dopo che eullm ha
    /// liberato la VRAM con /api/unload — vedi
    /// AppState::swap_embeddings_to_gpu. Richiede la feature "cuda"
    /// compilata (stesso limite di load()) — garantito da
    /// Settings::load(), che fallisce all'avvio se swap_during_ingestion è
    /// attivo su una build senza CUDA.
    pub fn load_gpu_for_ingestion(model_id: &str) -> Result<Self> {
        let device = Self::try_cuda_device(0)?;
        let mut svc = Self::load_on(model_id, &device)?;
        svc.device_status = DeviceStatus::Gpu;
        Ok(svc)
    }

    fn load_on(model_id: &str, device: &Device) -> Result<Self> {
        // Resolution order:
        //   1. Explicit local directory (model_id starts with "/" or "./")
        //   2. HF hub local cache (~/.cache/huggingface/hub) — bypasses HTTP entirely,
        //      immune to malformed https_proxy / HF_ENDPOINT env vars
        //   3. Download via hf_hub API (last resort, requires working network)
        let local = Path::new(model_id);
        let (config_path, tokenizer_path, weight_files) = if local.is_dir() {
            tracing::info!(model_id, "carico embedding da directory locale");
            resolve_model_dir(local)?
        } else if let Some(cache_dir) = find_in_hf_local_cache(model_id) {
            tracing::info!(model_id, cache = %cache_dir.display(), "trovato nella cache HF locale");
            resolve_model_dir(&cache_dir)?
        } else {
            bail!(
                "modello '{}' non trovato in cache locale ({}).\n\
                 Il download avviene all'avvio: se questo errore persiste,\n\
                 scaricalo manualmente con: huggingface-cli download {}",
                model_id,
                hf_cache_base().display(),
                model_id
            )
        };

        let config: BertConfig =
            serde_json::from_reader(std::fs::File::open(&config_path).context("open config")?)
                .context("deserialize BertConfig")?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?;

        tracing::info!(model_id, "carico pesi embedding (~2.3 GB)…");
        // bge-m3 = XLM-RoBERTa: weight keys prefissati con "roberta."
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_files, DType::F32, device)
                .context("VarBuilder mmap")?
        };
        let model = BertModel::load(vb, &config).context("BertModel::load")?;

        tracing::info!(model_id, "embedding model pronto su {device:?}");
        // device_status è un placeholder: il chiamante (load(), load_cpu_parked(),
        // load_gpu_for_ingestion()) lo sovrascrive sempre subito dopo, in base a
        // quale device/percorso ha effettivamente prodotto questo Self.
        Ok(Self {
            model,
            tokenizer,
            device: device.clone(),
            device_status: DeviceStatus::CpuByConfig,
            model_id: model_id.to_owned(),
        })
    }

    /// Embedding di un singolo testo, L2-normalizzato. dim=1024.
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let mut batch = self.embed_batch(&[text])?;
        Ok(batch.remove(0))
    }

    /// Embedding batch, L2-normalizzato. Chunka automaticamente per batch_size.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let bs = if device_is_cuda(&self.device) { GPU_BATCH } else { CPU_BATCH };
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(bs) {
            let mut rows = self.embed_batch(chunk).or_else(|e| {
                if device_is_cuda(&self.device) {
                    tracing::warn!("OOM CUDA, riprovo su CPU: {e:#}");
                    self.embed_batch_on(chunk, &Device::Cpu)
                } else {
                    Err(e)
                }
            })?;
            out.append(&mut rows);
        }
        Ok(out)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.embed_batch_on(texts, &self.device)
    }

    fn embed_batch_on(&self, texts: &[&str], device: &Device) -> Result<Vec<Vec<f32>>> {
        // Tokenizzazione con padding al testo più lungo del batch
        let encodings = self
            .tokenizer
            .encode_batch(
                texts.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                true, // add_special_tokens
            )
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;

        let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);
        if max_len == 0 {
            bail!("tokenizzazione vuota");
        }
        let n = texts.len();

        // Padding a max_len e flatten per Tensor::from_vec
        let mut ids: Vec<u32> = Vec::with_capacity(n * max_len);
        let mut type_ids: Vec<u32> = Vec::with_capacity(n * max_len);
        let mut masks: Vec<u32> = Vec::with_capacity(n * max_len);

        for enc in &encodings {
            let pad = max_len - enc.get_ids().len();
            ids.extend_from_slice(enc.get_ids());
            ids.extend(std::iter::repeat(0).take(pad));
            type_ids.extend_from_slice(enc.get_type_ids());
            type_ids.extend(std::iter::repeat(0).take(pad));
            masks.extend_from_slice(enc.get_attention_mask());
            masks.extend(std::iter::repeat(0).take(pad));
        }

        let input_ids = Tensor::from_vec(ids, (n, max_len), device)?;
        let token_type_ids = Tensor::from_vec(type_ids, (n, max_len), device)?;
        let attention_mask = Tensor::from_vec(masks, (n, max_len), device)?;

        // Forward pass → [batch, seq_len, hidden_dim]
        let hidden =
            self.model.forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

        // CLS pooling: [batch, seq_len, dim] → [batch, dim]
        let cls = hidden.narrow(1, 0, 1)?.squeeze(1)?;

        // L2 normalizzazione lungo dim=1
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = cls.broadcast_div(&norm)?;

        // Sposta su CPU e converti
        let cpu = normalized.to_device(&Device::Cpu)?;
        Ok(cpu.to_vec2::<f32>()?)
    }
}

/// L2-normalizza un vettore di embedding in place.
#[allow(dead_code)]
pub fn l2_normalize(v: &mut Vec<f32>) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

fn device_is_cuda(dev: &Device) -> bool {
    #[cfg(feature = "cuda")]
    {
        matches!(dev, Device::Cuda(_))
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = dev;
        false
    }
}

fn resolve_model_dir(dir: &Path) -> Result<(PathBuf, PathBuf, Vec<PathBuf>)> {
    let config = dir.join("config.json");
    let tokenizer = dir.join("tokenizer.json");
    if !config.exists() {
        anyhow::bail!("config.json non trovato in {}", dir.display());
    }
    if !tokenizer.exists() {
        anyhow::bail!("tokenizer.json non trovato in {}", dir.display());
    }
    let weights = collect_local_safetensors(dir)?;
    Ok((config, tokenizer, weights))
}

fn hf_cache_base() -> PathBuf {
    // Mirror huggingface-hub priority order:
    // HUGGINGFACE_HUB_CACHE > HF_HOME/hub > XDG_CACHE_HOME/huggingface/hub > ~/.cache/huggingface/hub
    if let Ok(v) = std::env::var("HUGGINGFACE_HUB_CACHE") {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return p;
        }
    }
    if let Ok(v) = std::env::var("HF_HOME") {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return p.join("hub");
        }
    }
    if let Ok(v) = std::env::var("XDG_CACHE_HOME") {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return p.join("huggingface").join("hub");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("huggingface").join("hub");
    }
    PathBuf::from(".")
}

/// Verifica che la directory contenga almeno un file safetensors di dimensione ragionevole.
/// Scarta file spazzatura (es. download interrotti, 0-byte, 29-byte, ecc.).
fn model_weights_valid(dir: &Path) -> bool {
    const MIN_BYTES: u64 = 10 * 1024 * 1024; // 10 MB — bge-m3 è ~2.3 GB
    let single = dir.join("model.safetensors");
    if single.exists() {
        return std::fs::metadata(&single).map(|m| m.len() >= MIN_BYTES).unwrap_or(false);
    }
    // Sharded: basta che l'index.json esista e sia non-triviale (i shard vengono validati al mmap)
    let idx = dir.join("model.safetensors.index.json");
    std::fs::metadata(&idx).map(|m| m.len() > 200).unwrap_or(false)
}

fn find_in_hf_local_cache(model_id: &str) -> Option<PathBuf> {
    // 1. HF hub layout: <cache>/models--BAAI--bge-m3/snapshots/<sha>/
    let model_slug = format!("models--{}", model_id.replace('/', "--"));
    let snapshots = hf_cache_base().join(&model_slug).join("snapshots");
    tracing::debug!(path = %snapshots.display(), "ricerca snapshot HF locale");
    if let Ok(rd) = std::fs::read_dir(&snapshots) {
        if let Some(p) = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("config.json").exists() && model_weights_valid(p))
            .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
        {
            return Some(p);
        }
    }

    // 2. sentence-transformers layout: ~/.cache/torch/sentence_transformers/BAAI_bge-m3/
    let st_slug = model_id.replace('/', "_");
    if let Ok(home) = std::env::var("HOME") {
        let st_path = PathBuf::from(home)
            .join(".cache")
            .join("torch")
            .join("sentence_transformers")
            .join(&st_slug);
        if st_path.join("config.json").exists() && model_weights_valid(&st_path) {
            tracing::debug!(path = %st_path.display(), "trovato in cache sentence-transformers");
            return Some(st_path);
        }
    }

    // 3. ~/.eullm/models/<basename>/ — dove bootstrap::ensure_embedding_model salva i pesi
    let dl_dir = download_target_dir(model_id);
    if dl_dir.join("config.json").exists() && model_weights_valid(&dl_dir) {
        tracing::debug!(path = %dl_dir.display(), "trovato in direct-download dir");
        return Some(dl_dir);
    }

    None
}

fn collect_local_safetensors(dir: &Path) -> Result<Vec<PathBuf>> {
    let single = dir.join("model.safetensors");
    if single.exists() {
        return Ok(vec![single]);
    }
    let idx_path = dir.join("model.safetensors.index.json");
    if idx_path.exists() {
        #[derive(serde::Deserialize)]
        struct Idx {
            weight_map: std::collections::HashMap<String, String>,
        }
        let idx: Idx = serde_json::from_reader(std::fs::File::open(&idx_path)?)
            .context("parse safetensors.index.json")?;
        let mut names: Vec<String> = idx.weight_map.into_values().collect();
        names.sort();
        names.dedup();
        return Ok(names.into_iter().map(|n| dir.join(n)).collect());
    }
    anyhow::bail!("nessun file safetensors trovato in {}", dir.display())
}

/// Directory dove bootstrap salva i file del modello embedding.
/// Usa I3K_DATA_DIR se impostato, altrimenti ~/.eullm (default storico).
pub fn download_target_dir(model_id: &str) -> PathBuf {
    let basename = model_id.rsplit('/').next().unwrap_or(model_id);
    let root = std::env::var("I3K_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".eullm")
        });
    root.join("models").join(basename)
}

#[cfg(test)]
mod tests {
    // Entrambi i test sotto sono #[cfg(not(feature = "cuda"))] — con "cuda"
    // attiva questo modulo è vuoto, l'import andrebbe gated allo stesso modo.
    #[cfg(not(feature = "cuda"))]
    use super::*;

    // Solo per build SENZA feature "cuda" (il caso normale di questo sandbox):
    // require_gpu=true deve fallire SUBITO, prima di toccare qualunque file
    // modello — non deve mai degradare in silenzio su CPU (5a, CLAUDE.md
    // REGOLA ZERO — "onestà sullo stato"). Un model_id inesistente basta a
    // dimostrare che l'errore arriva dal guard require_gpu e non da un
    // successivo tentativo di caricamento pesi.
    #[cfg(not(feature = "cuda"))]
    #[test]
    fn require_gpu_fails_hard_without_cuda_feature() {
        let result = EmbeddingService::load("modello/che-non-esiste-di-sicuro", true);
        let Err(err) = result else {
            panic!("deve fallire, non degradare su CPU");
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("REQUIRE_GPU") && msg.contains("cuda"),
            "messaggio d'errore non spiega la causa (REQUIRE_GPU + cuda): {msg}"
        );
    }

    // Senza require_gpu, la stessa build (niente feature "cuda") deve usare la
    // CPU per scelta di build — non è un fallback, quindi l'unico modo di
    // osservarlo da fuori load() è il fatto che NON fallisca per questo motivo
    // (l'errore che riceve qui viene dal model_id inventato, non da un guard
    // GPU) e che device_label/device_status lo marchino CpuByConfig, mai
    // CpuFallback, quando il caricamento riesce.
    #[cfg(not(feature = "cuda"))]
    #[test]
    fn cpu_by_config_is_not_reported_as_fallback_message() {
        let result = EmbeddingService::load("modello/che-non-esiste-di-sicuro", false);
        let Err(err) = result else {
            panic!("model_id inventato, deve comunque fallire il load pesi");
        };
        let msg = format!("{err:#}");
        assert!(
            !msg.contains("REQUIRE_GPU"),
            "senza require_gpu l'errore non deve menzionare REQUIRE_GPU: {msg}"
        );
    }
}
