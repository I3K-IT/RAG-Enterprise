//! Source formatting for API responses.
//! Mirrors Python _format_sources / _format_chunks (MAPPA §5, step 13).
//! Community: display_score = similarity (no reranker yet).

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Source {
    pub document_id: String,
    pub filename: String,
    pub chunk_index: usize,
    pub similarity: f32,
    pub text: String,
}
