//! Embedding service: BAAI/bge-m3 via Candle (in-process, GPU/CPU).
//!
//! Parità con Python (MAPPA §8):
//! - Modello: BAAI/bge-m3, dim 1024
//! - normalize_embeddings = true su query E documenti
//! - Pooling: CLS (primo token)
//! - Nessun prefisso "query:" / "passage:"
//! - GPU per batch indexing, ritorno a CPU dopo (libera VRAM all'LLM)
//! - Fallback CPU su CUDA OOM
//!
//! Golden-test da eseguire in Fase 1:
//! 50 coppie (query, doc) → cosine vs Python output, delta < 1e-3.

// TODO Fase 1: implementazione completa con candle-transformers + tokenizers

pub struct EmbeddingService {
    // model: BertModel,
    // tokenizer: Tokenizer,
    // device: Device,
}

impl EmbeddingService {
    pub fn load(_model_id: &str) -> anyhow::Result<Self> {
        // TODO Fase 1: load bge-m3 weights via candle-transformers
        // hf_hub download → BertModel::load → Tokenizer::from_pretrained
        todo!("load bge-m3 via Candle — Fase 1")
    }

    /// Embed a single query (CPU, normalized).
    pub fn embed_text(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        // CLS pooling → L2-normalize
        todo!("embed_text — Fase 1")
    }

    /// Embed a batch (GPU if available, returns to CPU after, fallback on OOM).
    pub fn embed_texts(&self, _texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        // Move to GPU, embed, move back to CPU
        todo!("embed_texts — Fase 1")
    }
}

/// L2-normalize an embedding vector in place.
pub fn l2_normalize(v: &mut Vec<f32>) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}
