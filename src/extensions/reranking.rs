//! Reranking extension point.
//!
//! Scaffolding only, not wired into api/query.rs — there is no reranking
//! here. Signature grounded in the real candidate type
//! (`SearchHit`, same as retrieval.rs) so a future reranker can be dropped
//! in against retrieval's real output rather than an invented type.

use anyhow::Result;
use async_trait::async_trait;

use crate::rag::vector_store::SearchHit;

#[async_trait]
pub trait Reranker: Send + Sync {
    /// Takes the retrieved candidates plus the original question (rerankers
    /// need it — they score query/passage pairs) and returns a re-ordered
    /// top-k. The no-op default returns `candidates` unchanged.
    async fn rerank(&self, question: &str, candidates: Vec<SearchHit>) -> Result<Vec<SearchHit>>;
}

pub struct NoOpReranker;

#[async_trait]
impl Reranker for NoOpReranker {
    async fn rerank(&self, _question: &str, candidates: Vec<SearchHit>) -> Result<Vec<SearchHit>> {
        Ok(candidates)
    }
}
