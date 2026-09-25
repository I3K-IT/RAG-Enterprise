//! Generic extension points: interfaces a crate that depends on this one
//! can implement and register, without forking it.
//!
//! This crate ships only default implementations — some genuinely no-op
//! (reranking, evidence, structured knowledge: there are no such features
//! here), some not: ingestion's chunk enricher wraps the heading injection
//! this crate already ships, and query routing's default wraps its single
//! real retrieval path rather than standing in for a feature that does not
//! exist. Either way the rule is the same: **no proprietary code or license
//! logic in this crate, ever** — only interfaces, and defaults that leave
//! this binary's behavior unchanged when nothing else is registered.

pub mod api;
pub mod evidence;
pub mod ingestion;
pub mod knowledge;
pub mod reranking;
pub mod retrieval;
pub mod routing;

pub use evidence::{EvidenceCheck, EvidenceLayer, NoOpEvidenceLayer};
pub use ingestion::{ChunkEnricher, DefaultChunkEnricher};
pub use knowledge::{NoOpStructuredKnowledgeProvider, StructuredKnowledgeProvider, StructuredKnowledgeResult};
pub use reranking::{NoOpReranker, Reranker};
pub use retrieval::{DefaultRetrieval, RetrievalStrategy};
pub use routing::{DefaultQueryPlanner, QueryPlanner, QueryRoute};

use std::sync::Arc;

/// Every per-request extension point a downstream binary can register, bundled
/// into one struct stored in `AppState`. `ExtensionRegistry::default()` —
/// what the Community binary itself always uses — must leave Community's
/// existing behavior unchanged; that invariant is each default impl's job,
/// not this struct's. The API router extension point is deliberately NOT
/// here — see `extensions::api`'s doc comment for why.
#[derive(Clone)]
pub struct ExtensionRegistry {
    pub chunk_enricher: Arc<dyn ChunkEnricher>,
    pub structured_knowledge: Arc<dyn StructuredKnowledgeProvider>,
    pub query_planner: Arc<dyn QueryPlanner>,
    pub retrieval: Arc<dyn RetrievalStrategy>,
    pub reranker: Arc<dyn Reranker>,
    pub evidence: Arc<dyn EvidenceLayer>,
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self {
            chunk_enricher: Arc::new(DefaultChunkEnricher),
            structured_knowledge: Arc::new(NoOpStructuredKnowledgeProvider),
            query_planner: Arc::new(DefaultQueryPlanner),
            retrieval: Arc::new(DefaultRetrieval),
            reranker: Arc::new(NoOpReranker),
            evidence: Arc::new(NoOpEvidenceLayer),
        }
    }
}
