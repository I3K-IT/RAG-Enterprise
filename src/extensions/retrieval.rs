//! Retrieval strategy extension point — I3K_RAG_Pro_Open_Core_Architecture.md
//! (rag-enterprise-pro, private repo) section 6.4.
//!
//! Signature grounded in the real call site (api/query.rs::prepare — the
//! only place Community does retrieval today: a single
//! `VectorStore::search` call), but NOT yet wired in there. Community's
//! query path stays untouched for this phase — see that architecture
//! document section 5.3/23 on minimizing risk this early. Wire this in
//! when Pro's hybrid/BM25/contextual retrieval (section 17.3) actually
//! needs to replace the single vector search, not before.

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
