//! Evidence extension point — I3K_RAG_Pro_Open_Core_Architecture.md
//! (rag-enterprise-pro, private repo) section 6.6.
//!
//! Pure scaffolding, deliberately: the document is explicit that the
//! claim -> fact -> NLI verification pipeline must NOT be built yet ("Non
//! implementare ancora un sistema post-generation claim → fact → NLI.
//! L'architettura di verification verrà definita separatamente.", and
//! again in section 22's "cosa NON fare"). This trait exists only so the
//! shape of the hook is visible; it has no real call site anywhere yet.

use anyhow::Result;
use async_trait::async_trait;

pub struct EvidenceCheck {
    pub verified: bool,
    pub note: Option<String>,
}

#[async_trait]
pub trait EvidenceLayer: Send + Sync {
    /// `generated_answer` against whatever sources were actually used.
    /// The no-op default always reports unverified/no-op — it must never be
    /// read as "verified true", since no verification runs at all yet.
    async fn check(&self, generated_answer: &str) -> Result<EvidenceCheck>;
}

pub struct NoOpEvidenceLayer;

#[async_trait]
impl EvidenceLayer for NoOpEvidenceLayer {
    async fn check(&self, _generated_answer: &str) -> Result<EvidenceCheck> {
        Ok(EvidenceCheck { verified: false, note: Some("evidence layer not implemented".into()) })
    }
}
