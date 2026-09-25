//! Retrieval strategy extension point.
//!
//! Signature grounded in the real call site (api/query.rs::prepare — the
//! only place retrieval happens: a single `VectorStore::search` call), but
//! NOT wired in there yet, to leave the query path untouched. Wire it in
//! when an implementation actually needs to replace the single vector
//! search, not before.

use anyhow::Result;
use async_trait::async_trait;

use crate::rag::vector_store::{SearchHit, VectorStore};

#[async_trait]
pub trait RetrievalStrategy: Send + Sync {
    async fn retrieve(&self, qdrant: &dyn VectorStore, query_vec: Vec<f32>) -> Result<Vec<SearchHit>>;
}

/// Community's own current behavior: one vector search, same top_k/threshold
/// as `rag::retrieval::{TOP_K, RELEVANCE_THRESHOLD}`.
pub struct DefaultRetrieval;

#[async_trait]
impl RetrievalStrategy for DefaultRetrieval {
    async fn retrieve(&self, qdrant: &dyn VectorStore, query_vec: Vec<f32>) -> Result<Vec<SearchHit>> {
        qdrant
            .search(
                query_vec,
                crate::rag::retrieval::TOP_K,
                Some(crate::rag::retrieval::RELEVANCE_THRESHOLD),
            )
            .await
    }
}
