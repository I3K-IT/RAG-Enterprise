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
    /// Free-form structured fields carried through untouched. Always None
    /// here; the slot exists so the payload schema stays interchangeable with
    /// producers that do populate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_fields: Option<serde_json::Value>,

    // ── Source Provenance Foundation ────────────────────────────────────
    // Infrastructural provenance only (locator + stable id) — NOT claim-level
    // attribution, evidence IDs, sentence citations, highlighting or NLI
    // verification: those stay Pro-roadmap items, deliberately out of scope
    // here. All Option so points written before these fields existed still
    // deserialize (see clients/qdrant_store.rs::search) without forcing a
    // re-ingestion.
    /// Byte-offset span `[source_start, source_end)` of this chunk within the
    /// extracted source text (before chunking) — the universal locator,
    /// available for every format. See rag::chunker::Chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_end: Option<usize>,
    /// 1-based page range this chunk overlaps — PDF only (native or OCR via
    /// documents::ocr::ocr_pdf), via documents::parser::pages_for_range.
    /// None for every other format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_end: Option<u32>,
    /// Deterministic, versioned citation identifier — see
    /// rag::chunker::provenance_id. Anchored to the uploaded file's own
    /// sha256, not to `document_id` (a fresh UUID per upload): stays stable
    /// across re-ingesting the same file content under the same chunking
    /// configuration, and visibly changes if that configuration changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_id: Option<String>,
}

/// The result of a vector similarity search.
#[derive(Debug)]
pub struct SearchHit {
    pub similarity: f32,
    pub payload: ChunkPayload,
}

/// Interface to the vector store.
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
