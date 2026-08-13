//! Document metadata in SQLite (`documents` table).
//! INVARIANT: delete touches Qdrant FIRST, then SQLite.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DocumentRow {
    pub id: String,
    pub filename: String,
    pub upload_date: String,
    pub page_count: Option<i64>,
    pub doc_type: String,
    pub chunk_count: i64,
    pub is_deleted: i64,
}

pub async fn insert(
    pool: &SqlitePool,
    id: &str,
    filename: &str,
    page_count: Option<u32>,
    doc_type: &str,
    chunk_count: usize,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO documents (id, filename, upload_date, page_count, doc_type, chunk_count)
         VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(id)
    .bind(filename)
    .bind(now)
    .bind(page_count.map(|n| n as i64))
    .bind(doc_type)
    .bind(chunk_count as i64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_active(pool: &SqlitePool) -> Result<Vec<DocumentRow>> {
    let rows = sqlx::query_as::<_, DocumentRow>(
        "SELECT id, filename, upload_date, page_count, doc_type, chunk_count, is_deleted
         FROM documents WHERE is_deleted = 0 ORDER BY upload_date DESC"
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn soft_delete(pool: &SqlitePool, document_id: &str) -> Result<bool> {
    let affected = sqlx::query(
        "UPDATE documents SET is_deleted = 1 WHERE id = ? AND is_deleted = 0"
    )
    .bind(document_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<DocumentRow>> {
    let rows = sqlx::query_as::<_, DocumentRow>(
        "SELECT id, filename, upload_date, page_count, doc_type, chunk_count, is_deleted
         FROM documents ORDER BY upload_date DESC"
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn find_by_id(pool: &SqlitePool, document_id: &str) -> Result<Option<DocumentRow>> {
    let row = sqlx::query_as::<_, DocumentRow>(
        "SELECT id, filename, upload_date, page_count, doc_type, chunk_count, is_deleted
         FROM documents WHERE id = ?"
    )
    .bind(document_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
