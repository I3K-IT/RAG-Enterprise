//! Query routing extension point.
//!
//! Scaffolding only, same treatment as knowledge.rs and evidence.rs: there
//! is only one execution path here, so there is nothing to route between.
//! This trait exists only so the shape of the hook is visible; it has no
//! real call site anywhere (query.rs always does a single vector search,
//! unconditionally).
//!
//! `DefaultQueryPlanner` always returning `Semantic` is not a placeholder,
//! though: it is Community's actual, complete current behavior (there is
//! no other path to choose between yet), same spirit as `DefaultRetrieval`.

use anyhow::Result;
use async_trait::async_trait;

/// The execution paths a planner can choose between. Only `Semantic` exists
/// here; a `Combined` route would have to be a real execution path of its
/// own, not a relabelled `Semantic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryRoute {
    Semantic,
    Structured,
    Combined,
}

#[async_trait]
pub trait QueryPlanner: Send + Sync {
    async fn plan(&self, question: &str) -> Result<QueryRoute>;
}

pub struct DefaultQueryPlanner;

#[async_trait]
impl QueryPlanner for DefaultQueryPlanner {
    async fn plan(&self, _question: &str) -> Result<QueryRoute> {
        Ok(QueryRoute::Semantic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_planner_always_routes_semantic() {
        let route = DefaultQueryPlanner.plan("una domanda qualunque").await.unwrap();
        assert_eq!(route, QueryRoute::Semantic, "Community has only one real retrieval path today");
    }
}
