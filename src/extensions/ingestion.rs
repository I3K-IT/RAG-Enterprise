//! Chunk enrichment extension point — I3K_RAG_Pro_Open_Core_Architecture.md
//! (rag-enterprise-pro, private repo) section 6.1.
//!
//! Hook: between chunking+provenance and embedding/indexing (see
//! api/documents.rs and bench.rs — both call `state.extensions.chunk_enricher`
//! where they used to call `rag::chunker::inject_heading_context` directly).
//! Pro's Contextual Retrieval enricher registers here.

use anyhow::Result;
use async_trait::async_trait;

use crate::rag::chunker::Chunk;

/// Produces the text that actually gets embedded and stored for each chunk,
/// given the chunk's own text plus the surrounding document for context.
/// Returns one String per chunk, same order as `chunks` — this is exactly
/// `rag::chunker::inject_heading_context`'s existing signature, promoted to
/// a trait so Pro can register a different implementation without Community
/// depending on Pro's code.
#[async_trait]
pub trait ChunkEnricher: Send + Sync {
    async fn enrich(&self, document_text: &str, chunks: &[Chunk]) -> Result<Vec<String>>;
}

/// Community's own default: the regex-based structural-heading injection
/// already shipped and tested (see rag::chunker::{detect_headings,
/// inject_heading_context}). Not a no-op — Community already has real,
/// generic (non-proprietary) chunk enrichment — but registering a Pro
/// enricher here must still leave Community's own behavior identical when
/// no Pro enricher is registered, per the architecture document's rule.
pub struct DefaultChunkEnricher;

#[async_trait]
impl ChunkEnricher for DefaultChunkEnricher {
    async fn enrich(&self, document_text: &str, chunks: &[Chunk]) -> Result<Vec<String>> {
        Ok(crate::rag::chunker::inject_heading_context(document_text, chunks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_enricher_matches_calling_inject_heading_context_directly() {
        let text = "Article 1\nTitle\n\n1. Body text here.";
        let chunks = crate::rag::chunker::split_text(text);
        let direct = crate::rag::chunker::inject_heading_context(text, &chunks);

        let via_trait = DefaultChunkEnricher.enrich(text, &chunks).await.unwrap();

        assert_eq!(direct, via_trait, "the trait wrapper must not change the existing behavior");
    }
}
