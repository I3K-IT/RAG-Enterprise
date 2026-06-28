//! Vector retrieval — wraps QdrantStore.
//! Community: pure vector search (no hybrid/BM25 — those are Fase 2 Pro).
//!
//! Parameters (MAPPA §5):
//! top_k = 15, relevance_threshold = 0.30

pub const TOP_K: u64 = 15;
pub const RELEVANCE_THRESHOLD: f32 = 0.30;

// TODO Fase 1: wire up QdrantStore + EmbeddingService
