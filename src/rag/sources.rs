//! Source formatting for API responses.
//! display_score = similarity.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Source {
    pub document_id: String,
    pub filename: String,
    pub chunk_index: usize,
    pub similarity: f32,
    pub text: String,
    // Source Provenance Foundation — infrastructural locator only, see
    // rag::vector_store::ChunkPayload for what each field means and why
    // they're all optional (absent on sources predating this feature).
    // source_start_byte/source_end_byte are BYTE offsets, not char offsets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_start_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_end_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_id: Option<String>,
}
