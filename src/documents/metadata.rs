//! Document metadata (stored in SQLite alongside Qdrant vectors).
//! Tracks: document_id, filename, upload_date, page_count, doc_type, status.
//!
//! INVARIANTE: delete / reindex devono toccare Qdrant PRIMA, poi SQLite.
//! Un solo punto di ingresso — mai bypassare.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentMeta {
    pub document_id: String,
    pub filename: String,
    pub upload_date: DateTime<Utc>,
    pub page_count: Option<u32>,
    pub doc_type: String,
    pub chunk_count: usize,
}
