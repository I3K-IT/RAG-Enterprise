//! Volume-cap + ED25519 licence verification (Fase 3 — Pro).
//!
//! Licence payload: { customer, pages_limit (null = unlimited), expires, signature }
//! Signature: ED25519 (ed25519-dalek). Public key bundled in binary.
//! check_page_limit() called BEFORE every ingest — the single gating point.
//!
//! Community: no gating (pages_limit = unlimited, no licence required).
//! Pro trial: hardcoded page cap, no licence file needed.
//! Pro licensed: licence file raises/removes the cap.
//!
//! TODO Fase 3: implement licence loading + signature verification + page counter.

/// Check if the page limit allows ingesting `new_pages` more pages.
/// Community always returns Ok.
pub fn check_page_limit(_current_pages: u64, _new_pages: u64) -> anyhow::Result<()> {
    // Community: unlimited
    Ok(())
}
