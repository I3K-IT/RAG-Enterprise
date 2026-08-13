//! Document metadata (stored in SQLite alongside Qdrant vectors).
//! Tracks: document_id, filename, upload_date, page_count, doc_type, status.
//!
//! INVARIANT: delete and reindex must touch Qdrant FIRST, then SQLite.
//! A single entry point — never bypass it.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentMeta {
    pub document_id: String,
    pub filename: String,
    pub upload_date: DateTime<Utc>,
    pub page_count: Option<u32>,
    pub doc_type: String,
    pub chunk_count: usize,
}
