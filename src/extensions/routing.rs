//! Query routing extension point — I3K_RAG_Pro_Open_Core_Architecture.md
//! (rag-enterprise-pro, private repo) section 6.3.
//!
//! Scaffolding only, same treatment as knowledge.rs and evidence.rs: the
//! real router (deciding between the SEMANTIC/STRUCTURED/COMBINED execution
//! paths) is explicitly PHASE 9 in that document's section 24/17.2, well
//! after this phase — building real routing logic now would be doing
//! PHASE 9's work out of order. This trait exists only so the shape of the
//! hook is visible; it has no real call site anywhere yet (query.rs always
//! does a single vector search, unconditionally).
//!
//! `DefaultQueryPlanner` always returning `Semantic` is not a placeholder,
//! though: it is Community's actual, complete current behavior (there is
//! no other path to choose between yet), same spirit as `DefaultRetrieval`.

use anyhow::Result;
use async_trait::async_trait;

/// The three execution paths named in that document's sections 6.3/17.2.
/// `Combined`, per section 17.2, "must be a real execution path if it is
/// introduced" — not yet introduced anywhere, Community or Pro.
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
