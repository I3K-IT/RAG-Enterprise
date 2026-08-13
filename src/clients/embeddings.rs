//! Embedding service: BAAI/bge-m3 via Candle (in-process, GPU/CPU).
//!
//! Parity with the Python implementation (embeddings_service.py):
//! - SentenceTransformer("BAAI/bge-m3", normalize_embeddings=True)
//! - Architecture: XLM-RoBERTa → weight keys prefixed with "roberta"
//! - Pooling: CLS (index 0 of the sequence)
//! - L2 normalisation on queries AND documents
//! - GPU batch=4 (conservative: a 14b model plus bge-m3 is ~11 GB of 16 GB),
//!   CPU batch=2
//! - CUDA OOM → automatic per-batch CPU fallback
//!
//! Device selection (see load()): the CPU can be reached by two quite distinct
//! routes which must not be conflated, so DeviceStatus keeps them apart:
//!   - CpuByConfig: built without the "cuda" feature, where CPU is the
//!     expected choice.
//!   - CpuFallback: CUDA was requested but failed after the retries. That is a
//!     real degradation and must always be reported loudly (error! log, and
//!     exposed through GET /info).

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
/// CUDA init attempts (device plus weight loading) before settling for CPU.
const CUDA_LOAD_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// CUDA was requested (the "cuda" feature is compiled in) and succeeded.
    Gpu,
    /// CPU by build choice (the "cuda" feature is not compiled in) — not a fallback.
    CpuByConfig,
    /// CUDA requested but failed after CUDA_LOAD_ATTEMPTS attempts — a real degradation.
    CpuFallback,
    /// Deliberately on CPU between ingestion windows
    /// (EmbeddingsSettings::swap_during_ingestion=true). Not a degradation:
    /// outside ingestion the VRAM is reserved for eullm by design.
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
    /// Loads bge-m3. When the "cuda" feature is compiled in, it tries CUDA for
    /// CUDA_LOAD_ATTEMPTS attempts (backing off 1s then 2s between them)
    /// before falling back to CPU. `require_gpu=true`
    /// (EMBEDDINGS__REQUIRE_GPU) makes startup fail rather than degrade
    /// silently — an ingestion taking 17 minutes must never go unnoticed.
    pub fn load(model_id: &str, require_gpu: bool) -> Result<Self> {
        if !cfg!(feature = "cuda") {
            if require_gpu {
                bail!(
                    "EMBEDDINGS__REQUIRE_GPU=true but this binary was built without \
                     --features cuda: GPU embeddings are not possible. \
                     Rebuild with --features cuda (or ocr,cuda)."
                );
            }
            tracing::info!(model_id, "embeddings on CPU (built without the \"cuda\" feature)");
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
                    tracing::info!(model_id, attempt, "embedding model ready on GPU (CUDA)");
                    return Ok(svc);
                }
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = CUDA_LOAD_ATTEMPTS,
                        error = ?e,
                        "CUDA init for embeddings failed (attempt {attempt}/{CUDA_LOAD_ATTEMPTS})"
                    );
                    last_err = Some(e);
                    if attempt < CUDA_LOAD_ATTEMPTS {
                        std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                    }
                }
            }
        }
        let e = last_err.expect("at least one attempt ran in the loop above");

        if require_gpu {
            return Err(e.context(
                "EMBEDDINGS__REQUIRE_GPU=true: CUDA failed after every attempt — \
                 aborting startup rather than degrading to CPU",
            ));
        }

        tracing::error!(
            error = ?e,
            "\n================================================================\n\
             EMBEDDINGS FELL BACK TO CPU — CUDA failed after {CUDA_LOAD_ATTEMPTS} attempts.\n\
             Document ingestion will be MUCH slower (minutes rather than seconds).\n\
             The real cause is in the 'error' field above (full chain).\n\
             Set EMBEDDINGS__REQUIRE_GPU=true to fail immediately instead of degrading.\n\
             ================================================================"
        );
        let mut svc = Self::load_on(model_id, &Device::Cpu)
            .context("CPU fallback after CUDA failure")?;
        svc.device_status = DeviceStatus::CpuFallback;
        Ok(svc)
    }

    #[cfg(feature = "cuda")]
    fn try_cuda_device(ordinal: usize) -> Result<Device> {
        Device::new_cuda(ordinal).context("Device::new_cuda")
    }
    #[cfg(not(feature = "cuda"))]
    fn try_cuda_device(_ordinal: usize) -> Result<Device> {
        unreachable!("only called when the \"cuda\" feature is enabled")
    }

    pub fn device_status(&self) -> DeviceStatus {
        self.device_status
    }

    pub fn device_label(&self) -> &'static str {
        match self.device_status {
            DeviceStatus::Gpu => "gpu",
            DeviceStatus::CpuByConfig => "cpu (built without GPU support)",
            DeviceStatus::CpuFallback => "cpu (FALLBACK: CUDA failed at startup, see the log)",
            DeviceStatus::CpuParked => "cpu (parked — VRAM reserved for eullm outside ingestion)",
        }
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Loads bge-m3 on CPU with CpuParked status. Used both at startup when
    /// EMBEDDINGS__SWAP_DURING_INGESTION=true — the VRAM stays free for eullm
    /// until an ingestion begins — and to move back to CPU once an ingestion
    /// ends (see AppState::swap_embeddings_to_cpu).
    pub fn load_cpu_parked(model_id: &str) -> Result<Self> {
        let mut svc = Self::load_on(model_id, &Device::Cpu)?;
        svc.device_status = DeviceStatus::CpuParked;
        Ok(svc)
    }

    /// Loads bge-m3 on the GPU for the ingestion window, after eullm has
    /// released the VRAM through /api/unload — see
    /// AppState::swap_embeddings_to_gpu. Requires the "cuda" feature to be
    /// compiled in, the same constraint as load(), which Settings::load()
    /// guarantees by failing at startup when swap_during_ingestion is enabled
    /// on a build without CUDA.
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
            tracing::info!(model_id, cache = %cache_dir.display(), "found in the local HF cache");
            resolve_model_dir(&cache_dir)?
        } else {
            bail!(
                "model '{}' not found in the local cache ({}).\n\
                 It is downloaded at startup: if this error persists,\n\
                 fetch it manually with: huggingface-cli download {}",
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

        tracing::info!(model_id, "loading embedding weights (~2.3 GB)…");
        // bge-m3 = XLM-RoBERTa: weight keys prefissati con "roberta."
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_files, DType::F32, device)
                .context("VarBuilder mmap")?
        };
        let model = BertModel::load(vb, &config).context("BertModel::load")?;

        tracing::info!(model_id, "embedding model ready on {device:?}");
        // device_status is a placeholder: the caller — load(), load_cpu_parked()
        // or load_gpu_for_ingestion() — always overwrites it immediately after,
        // according to which device and path actually produced this Self.
        Ok(Self {
            model,
            tokenizer,
            device: device.clone(),
            device_status: DeviceStatus::CpuByConfig,
            model_id: model_id.to_owned(),
        })
    }

    /// Embeds a single text, L2-normalised. dim=1024.
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let mut batch = self.embed_batch(&[text])?;
        Ok(batch.remove(0))
    }

    /// Batch embedding, L2-normalised. Chunks automatically by batch size.
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
        // Tokenise, padding to the longest text in the batch.
        let encodings = self
            .tokenizer
            .encode_batch(
                texts.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                true, // add_special_tokens
            )
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;

        let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);
        if max_len == 0 {
            bail!("empty tokenisation");
        }
        let n = texts.len();

        // Pad to max_len and flatten for Tensor::from_vec.
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
        anyhow::bail!("config.json not found in {}", dir.display());
    }
    if !tokenizer.exists() {
        anyhow::bail!("tokenizer.json not found in {}", dir.display());
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

/// Checks that the directory holds at least one safetensors file of a
/// plausible size, rejecting junk such as interrupted downloads (0-byte,
/// 29-byte and the like).
fn model_weights_valid(dir: &Path) -> bool {
    const MIN_BYTES: u64 = 10 * 1024 * 1024; // 10 MB — bge-m3 is around 2.3 GB
    let single = dir.join("model.safetensors");
    if single.exists() {
        return std::fs::metadata(&single).map(|m| m.len() >= MIN_BYTES).unwrap_or(false);
    }
    // Sharded: it is enough that index.json exists and is non-trivial; the
    // shards themselves are validated when they are mmapped.
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

    // 3. ~/.eullm/models/<basename>/ — where bootstrap::ensure_embedding_model saves the weights
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
    anyhow::bail!("no safetensors file found in {}", dir.display())
}

/// Directory dove bootstrap salva i file del modello embedding.
/// Uses I3K_DATA_DIR when set, otherwise ~/.eullm (the historical default).
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
    // Both tests below are #[cfg(not(feature = "cuda"))]: with "cuda" enabled
    // this module is empty, so the import has to be gated the same way.
    #[cfg(not(feature = "cuda"))]
    use super::*;

    // For builds WITHOUT the "cuda" feature: require_gpu=true must fail
    // IMMEDIATELY, before touching any model file, and must never degrade
    // silently to CPU. A non-existent model_id is enough to show the error
    // comes from the require_gpu guard rather than from a later attempt to
    // load weights.
    #[cfg(not(feature = "cuda"))]
    #[test]
    fn require_gpu_fails_hard_without_cuda_feature() {
        let result = EmbeddingService::load("modello/che-non-esiste-di-sicuro", true);
        let Err(err) = result else {
            panic!("must fail, not degrade to CPU");
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("REQUIRE_GPU") && msg.contains("cuda"),
            "the error message does not explain the cause (REQUIRE_GPU + cuda): {msg}"
        );
    }

    // Without require_gpu, the same build (no "cuda" feature) must use the CPU
    // by build choice. That is not a fallback, so the only way to observe it
    // from outside load() is that it does NOT fail for this reason — the error
    // it gets here comes from the made-up model_id, not from a GPU guard — and
    // that device_label/device_status mark it CpuByConfig, never CpuFallback,
    // when loading succeeds.
    #[cfg(not(feature = "cuda"))]
    #[test]
    fn cpu_by_config_is_not_reported_as_fallback_message() {
        let result = EmbeddingService::load("modello/che-non-esiste-di-sicuro", false);
        let Err(err) = result else {
            panic!("made-up model_id, the weight load must fail regardless");
        };
        let msg = format!("{err:#}");
        assert!(
            !msg.contains("REQUIRE_GPU"),
            "without require_gpu the error must not mention REQUIRE_GPU: {msg}"
        );
    }
}
