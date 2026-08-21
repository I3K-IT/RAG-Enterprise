//! Generic extension points the Community core exposes for the Pro binary
//! to implement, per `I3K_RAG_Pro_Open_Core_Architecture.md`
//! (`rag-enterprise-pro`, a private repository — not this one) section 6.
//!
//! Community ships only default implementations here — some genuinely
//! no-op (reranking, evidence, structured knowledge — Community has no
//! such features today), some not: ingestion's chunk enricher wraps
//! Community's own already-shipped heading-injection logic, and query
//! routing's default wraps Community's own single real retrieval path
//! rather than standing in for a feature that doesn't exist yet. Either
//! way, the absolute rule from that document's section 7 applies: **no
//! proprietary Pro code or license logic in this crate, ever** — only
//! interfaces and defaults that leave this binary's own behavior
//! unchanged when nothing else is registered.

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

/// Every per-request extension point the Pro binary can register, bundled
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
