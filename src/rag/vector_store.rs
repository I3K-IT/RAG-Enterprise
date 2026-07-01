//! VectorStore trait — astrazione sul database vettoriale.
//!
//! Attualmente implementata da QdrantStore.
//! Progettata per permettere future implementazioni sqlite-vec/hnsw
//! (trial multi-OS senza dipendenza da Qdrant).
//!
//! INVARIANTE (CLAUDE.md): delete_document / reindex devono toccare SQLite E Qdrant.
//! Un solo punto di ingresso — mai bypassarlo.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Chunk salvato nel vector store.
/// Parità con Python (qdrant_connector.py): stessi campi nel payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPayload {
    pub document_id: String,
    pub chunk_index: usize,
    pub filename: String,
    pub upload_date: String,
    pub text: String,
    pub chunk_size: usize,
    pub document_type: String,
    /// Campi strutturati estratti (metadata pipeline — Fase 2 Pro).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_fields: Option<serde_json::Value>,
}

/// Risultato di una ricerca per similarità vettoriale.
#[derive(Debug)]
pub struct SearchHit {
    pub similarity: f32,
    pub payload: ChunkPayload,
}

/// Interfaccia verso il vector store.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Upsert embeddings con i relativi payload in batch da 1000.
    async fn upsert(&self, embeddings: &[Vec<f32>], payloads: &[ChunkPayload]) -> Result<()>;

    /// Ricerca per similarità vettoriale (top-k).
    async fn search(
        &self,
        query_vec: Vec<f32>,
        top_k: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<SearchHit>>;

    /// Cancella tutti i vettori di un documento.
    /// INVARIANTE: chiamare PRIMA di aggiornare SQLite.
    async fn delete_document(&self, document_id: &str) -> Result<()>;
}
