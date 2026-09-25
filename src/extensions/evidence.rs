//! Evidence extension point.
//!
//! Scaffolding only, deliberately: nothing checks an answer against its
//! sources yet. This trait exists only so the shape of the hook is
//! visible; it has no real call site anywhere.

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
