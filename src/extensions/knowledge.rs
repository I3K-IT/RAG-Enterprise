//! Structured knowledge extension point.
//!
//! Scaffolding only: there is no structured-extraction pipeline here, so no
//! real call site to ground an exact signature in — unlike
//! `ingestion::ChunkEnricher`. The interface should be revisited against the
//! first real implementation rather than treated as final.

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
