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
    /// `top_k` is the depth the request resolved to, already clamped - see
    /// `api::query::resolve_top_k`. It is deliberately a parameter rather than
    /// something an implementation reads from `TOP_K`: the request may ask for
    /// fewer or more, and the one call site today passes the resolved depth
    /// straight through. A strategy that reached for the constant instead would
    /// hand a caller who asked for 5 chunks a page of 15, which is a different
    /// answer and not "the same search" by another name.
    async fn retrieve(
        &self,
        qdrant: &dyn VectorStore,
        query_vec: Vec<f32>,
        top_k: u64,
    ) -> Result<Vec<SearchHit>>;
}

/// Community's own current behaviour: one vector search at the depth the caller
/// resolved, with the same threshold as `rag::retrieval::RELEVANCE_THRESHOLD`.
pub struct DefaultRetrieval;

#[async_trait]
impl RetrievalStrategy for DefaultRetrieval {
    async fn retrieve(
        &self,
        qdrant: &dyn VectorStore,
        query_vec: Vec<f32>,
        top_k: u64,
    ) -> Result<Vec<SearchHit>> {
        qdrant
            .search(query_vec, top_k, Some(crate::rag::retrieval::RELEVANCE_THRESHOLD))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::vector_store::ChunkPayload;
    use std::sync::Mutex;

    /// Records what the strategy asked the store for.
    struct RecordingStore {
        seen: Mutex<Vec<(u64, Option<f32>)>>,
    }

    #[async_trait]
    impl VectorStore for RecordingStore {
        async fn upsert(&self, _embeddings: &[Vec<f32>], _payloads: &[ChunkPayload]) -> Result<()> {
            Ok(())
        }

        async fn search(
            &self,
            _query_vec: Vec<f32>,
            top_k: u64,
            score_threshold: Option<f32>,
        ) -> Result<Vec<SearchHit>> {
            self.seen.lock().expect("lock").push((top_k, score_threshold));
            Ok(Vec::new())
        }

        async fn delete_document(&self, _document_id: &str) -> Result<()> {
            Ok(())
        }
    }

    /// The registry's invariant is that its defaults leave Community's
    /// behaviour unchanged, and the depth the caller resolved is part of that
    /// behaviour - the real call site passes it straight to `search`.
    #[tokio::test]
    async fn the_default_forwards_the_depth_the_caller_resolved() {
        for asked in [1u64, 5, 15, 50] {
            let store = RecordingStore { seen: Mutex::new(Vec::new()) };
            DefaultRetrieval
                .retrieve(&store, vec![0.0; 4], asked)
                .await
                .expect("retrieve");
            assert_eq!(
                store.seen.lock().expect("lock").as_slice(),
                &[(asked, Some(crate::rag::retrieval::RELEVANCE_THRESHOLD))],
                "asked for {asked} chunks"
            );
        }
    }

    /// The default must not quietly substitute the constant, which would be the
    /// one way it could fail while still looking like it worked.
    #[tokio::test]
    async fn the_default_never_substitutes_the_constant_depth() {
        let store = RecordingStore { seen: Mutex::new(Vec::new()) };
        DefaultRetrieval
            .retrieve(&store, vec![0.0; 4], 3)
            .await
            .expect("retrieve");
        let seen = store.seen.lock().expect("lock").clone();
        assert_eq!(seen[0].0, 3);
        assert_ne!(
            seen[0].0,
            crate::rag::retrieval::TOP_K,
            "3 must not come back as {}",
            crate::rag::retrieval::TOP_K
        );
    }
}
