//! Structured knowledge extension point — I3K_RAG_Pro_Open_Core_Architecture.md
//! (rag-enterprise-pro, private repo) section 6.2.
//!
//! Scaffolding only: Community has no structured-extraction pipeline
//! (entities/events/amounts/relationships/domain profiles) today, so there
//! is no real call site yet to ground an exact signature in — unlike
//! `ingestion::ChunkEnricher`. Pro's structured-extraction module (section
//! 17.1) is expected to be the first real implementation; this interface
//! should be revisited against that work rather than treated as final.

use anyhow::Result;
use async_trait::async_trait;

/// Placeholder for whatever a query needs from structured knowledge —
/// entities, events, amounts, relationships. Intentionally a permissive
/// bag of text (JSON-serialized, provider-defined shape) rather than a
/// concrete typed struct, until a real implementation exists to design
/// against.
pub struct StructuredKnowledgeResult {
    pub summary_text: String,
}

#[async_trait]
pub trait StructuredKnowledgeProvider: Send + Sync {
    /// Called during query execution. `None` means "nothing structured to
    /// add" — the default, and Community's only implementation today.
    async fn query(&self, question: &str) -> Result<Option<StructuredKnowledgeResult>>;
}

pub struct NoOpStructuredKnowledgeProvider;

#[async_trait]
impl StructuredKnowledgeProvider for NoOpStructuredKnowledgeProvider {
    async fn query(&self, _question: &str) -> Result<Option<StructuredKnowledgeResult>> {
        Ok(None)
    }
}
