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
}
