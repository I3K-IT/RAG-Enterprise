//! VectorStore trait — an abstraction over the vector database.
//!
//! Currently implemented by QdrantStore, and designed to leave room for
//! sqlite-vec or hnsw implementations later (a multi-OS trial with no Qdrant
//! dependency).
//!
//! INVARIANT: delete_document and reindex must touch BOTH SQLite and Qdrant.
//! There is a single entry point for that — never bypass it.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A chunk stored in the vector store.
/// Parity with the Python qdrant_connector.py: the same payload fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPayload {
    pub document_id: String,
    pub chunk_index: usize,
    pub filename: String,
    pub upload_date: String,
    pub text: String,
    pub chunk_size: usize,
    pub document_type: String,
    /// Extracted structured fields (metadata pipeline — Pro).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_fields: Option<serde_json::Value>,
}

/// The result of a vector similarity search.
#[derive(Debug)]
pub struct SearchHit {
    pub similarity: f32,
    pub payload: ChunkPayload,
}

/// Interfaccia verso il vector store.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Upserts embeddings with their payloads, in batches of 1000.
    async fn upsert(&self, embeddings: &[Vec<f32>], payloads: &[ChunkPayload]) -> Result<()>;

    /// Vector similarity search (top-k).
    async fn search(
        &self,
        query_vec: Vec<f32>,
        top_k: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<SearchHit>>;

    /// Cancella tutti i vettori di un documento.
    /// INVARIANT: call this BEFORE updating SQLite.
    async fn delete_document(&self, document_id: &str) -> Result<()>;
}
