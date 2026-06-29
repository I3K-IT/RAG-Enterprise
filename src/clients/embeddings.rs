//! Embedding service: BAAI/bge-m3 via Candle (in-process, GPU/CPU).
//!
//! Parità con Python (embeddings_service.py):
//! - SentenceTransformer("BAAI/bge-m3", normalize_embeddings=True)
//! - Architettura: XLM-RoBERTa → weight keys prefissati con "roberta"
//! - Pooling: CLS (indice 0 della sequenza)
//! - L2-normalizzazione su query E documenti
//! - GPU batch=4 (conservativo: 14b + bge-m3 ~11 GB su 16 GB), CPU batch=2
//! - OOM CUDA → fallback CPU automatico sul batch

use anyhow::{bail, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use hf_hub::api::sync::Api;
use std::path::PathBuf;
use tokenizers::Tokenizer;

pub const EMBED_DIM: usize = 1024;
const GPU_BATCH: usize = 4;
const CPU_BATCH: usize = 2;

pub struct EmbeddingService {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl EmbeddingService {
    /// Carica bge-m3 da HuggingFace hub (cache locale ~/.cache/huggingface/).
    /// Prova CUDA, poi ricade su CPU.
    pub fn load(model_id: &str) -> Result<Self> {
        let device = preferred_device();
        tracing::info!(model_id, "carico modello embedding su {device:?}");

        match Self::load_on(model_id, &device) {
            Ok(s) => Ok(s),
            Err(e) if device_is_cuda(&device) => {
                tracing::warn!("CUDA fallisce ({e:#}), ricarico su CPU");
                Self::load_on(model_id, &Device::Cpu)
            }
            Err(e) => Err(e),
        }
    }

    fn load_on(model_id: &str, device: &Device) -> Result<Self> {
        let api = Api::new().context("hf_hub init")?;
        let repo = api.model(model_id.to_owned());

        let config_path = repo.get("config.json").context("fetch config.json")?;
        let tokenizer_path = repo.get("tokenizer.json").context("fetch tokenizer.json")?;

        let config: BertConfig =
            serde_json::from_reader(std::fs::File::open(&config_path).context("open config")?)
                .context("deserialize BertConfig")?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?;

        tracing::info!(model_id, "recupero pesi (~2.3 GB)…");
        let weight_files = fetch_safetensors(&repo)?;

        // bge-m3 = XLM-RoBERTa: i weight keys sono prefissati con "roberta."
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_files, DType::F32, device)
                .context("VarBuilder mmap")?
        };
        let model = BertModel::load(vb.pp("roberta"), &config).context("BertModel::load")?;

        tracing::info!(model_id, "embedding model pronto su {device:?}");
        Ok(Self { model, tokenizer, device: device.clone() })
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
pub fn l2_normalize(v: &mut Vec<f32>) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

fn preferred_device() -> Device {
    #[cfg(feature = "cuda")]
    {
        match Device::new_cuda(0) {
            Ok(dev) => return dev,
            Err(e) => tracing::warn!("CUDA non disponibile: {e}"),
        }
    }
    Device::Cpu
}

fn device_is_cuda(dev: &Device) -> bool {
    #[cfg(feature = "cuda")]
    {
        return matches!(dev, Device::Cuda(_));
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = dev;
        false
    }
}

fn fetch_safetensors(repo: &hf_hub::api::sync::ApiRepo) -> Result<Vec<PathBuf>> {
    // Prova file singolo
    if let Ok(p) = repo.get("model.safetensors") {
        return Ok(vec![p]);
    }
    // Sharded: leggi l'indice per la lista esatta dei file
    if let Ok(idx_path) = repo.get("model.safetensors.index.json") {
        #[derive(serde::Deserialize)]
        struct Idx {
            weight_map: std::collections::HashMap<String, String>,
        }
        let idx: Idx = serde_json::from_reader(std::fs::File::open(idx_path)?)
            .context("parse safetensors.index.json")?;
        let mut names: Vec<String> = idx.weight_map.into_values().collect();
        names.sort();
        names.dedup();
        let mut files = Vec::with_capacity(names.len());
        for name in names {
            files.push(repo.get(&name).with_context(|| format!("shard {name}"))?);
        }
        return Ok(files);
    }
    bail!("nessun file safetensors trovato nel repo")
}
