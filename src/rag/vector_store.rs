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
    /// BYTE-offset span `[source_start_byte, source_end_byte)` of this
    /// chunk within the extracted source text (before chunking) — the
    /// universal locator, available for every format. Named `_byte`
    /// deliberately, not left implicit — see rag::chunker::Chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_start_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_end_byte: Option<usize>,
    /// 1-based page NUMBER range this chunk overlaps — PDF only (native or
    /// OCR via documents::ocr::ocr_pdf), via documents::parser::
    /// pages_for_range. None for every other format. Not a byte offset —
    /// no `_byte` suffix here on purpose, to keep the two units visually
    /// distinct.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_end: Option<u32>,
    /// Deterministic, versioned citation identifier — see
    /// rag::chunker::provenance_id. Anchored to the uploaded file's own
    /// sha256, not to `document_id` (a fresh UUID per upload): stays stable
    /// across re-ingesting the same file content under the same extraction
    /// and chunking configuration, and visibly changes if either changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_id: Option<String>,

    /// The enriched text actually embedded and searched — e.g. a heading
    /// prefix (Community's default enricher) or an LLM-generated context
    /// blurb (Pro's Contextual Retrieval) — when it differs from `text`.
    /// `None` means enrichment left this chunk unchanged (the common case:
    /// no heading detected, or, for older points, ingested before this
    /// field existed).
    ///
    /// `text` ALWAYS stays the chunk's real, unmodified source content —
    /// what a citation, source highlight or future evidence layer must
    /// show. `retrieval_text` exists only so query.rs can give the
    /// answering LLM the same enriched context the embedding was computed
    /// on, without that enrichment ever being presented as if it were the
    /// source itself. See rag-enterprise-pro's
    /// `I3K_RAG_Pro_Open_Core_Architecture.md` (private repo) section 14.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_text: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(retrieval_text: Option<&str>) -> ChunkPayload {
        ChunkPayload {
            document_id: "doc".into(),
            chunk_index: 0,
            filename: "f.txt".into(),
            upload_date: String::new(),
            text: "hello".into(),
            chunk_size: 5,
            document_type: "txt".into(),
            structured_fields: None,
            source_start_byte: None,
            source_end_byte: None,
            page_start: None,
            page_end: None,
            provenance_id: None,
            retrieval_text: retrieval_text.map(str::to_owned),
        }
    }

    #[test]
    fn retrieval_text_none_is_omitted_from_json() {
        let json = serde_json::to_value(payload(None)).unwrap();
        assert!(
            json.get("retrieval_text").is_none(),
            "skip_serializing_if must keep unenriched points identical to pre-Phase-6 ones"
        );
    }

    #[test]
    fn payload_without_retrieval_text_key_still_deserializes() {
        // Simulates a point written before this field existed — no
        // "retrieval_text" key in the stored JSON at all, not even null.
        let mut json = serde_json::to_value(payload(None)).unwrap();
        json.as_object_mut().unwrap().remove("retrieval_text");
        let p: ChunkPayload = serde_json::from_value(json).unwrap();
        assert_eq!(p.retrieval_text, None);
    }

    #[test]
    fn retrieval_text_round_trips_when_present() {
        let json = serde_json::to_value(payload(Some("[Article 42] hello"))).unwrap();
        let p: ChunkPayload = serde_json::from_value(json).unwrap();
        assert_eq!(p.retrieval_text.as_deref(), Some("[Article 42] hello"));
    }
}
