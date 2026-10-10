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
    // A plain INSERT, not INSERT OR IGNORE. `id` is a fresh UUIDv4 per upload and
    // the only unique column in the table, so there is nothing for OR IGNORE to
    // ignore: it could only ever mask the one failure that must not be masked.
    // An ignored insert returns Ok, the caller reports the upload as
    // successful, and the chunks are already in Qdrant - leaving the very
    // orphan this module's INVARIANT is about, with no row for the delete path
    // to reach. `create_conversation` next door already inserts plainly.
    sqlx::query(
        "INSERT INTO documents (id, filename, upload_date, page_count, doc_type, chunk_count)
         VALUES (?, ?, ?, ?, ?, ?)",
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

/// Active documents whose file name matches either of two LIKE patterns: the
/// narrowing step of the article lookup (rag::references::like_patterns),
/// which then checks each name exactly. Bounded: one article number is the
/// same number in every code of the library, never more.
pub async fn find_active_by_filename_like(
    pool: &SqlitePool,
    patterns: &[String; 2],
    limit: i64,
) -> Result<Vec<DocumentRow>> {
    let rows = sqlx::query_as::<_, DocumentRow>(
        "SELECT id, filename, upload_date, page_count, doc_type, chunk_count, is_deleted
         FROM documents WHERE is_deleted = 0 AND (filename LIKE ? OR filename LIKE ?)
         ORDER BY filename LIMIT ?"
    )
    .bind(&patterns[0])
    .bind(&patterns[1])
    .bind(limit)
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn migrated(dir: &std::path::Path) -> SqlitePool {
        let url = format!("sqlite://{}", dir.join("test.db").display());
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    /// An insert that does not happen must not look like one that did. The
    /// upload upserts its vectors to Qdrant before calling this, so an ignored
    /// insert reporting Ok is the orphan the delete path cannot reach.
    #[tokio::test]
    async fn a_repeated_document_id_is_an_error_not_a_silent_success() {
        let d = tempfile::tempdir().unwrap();
        let pool = migrated(d.path()).await;

        insert(&pool, "doc-1", "first.pdf", Some(3), "pdf", 7)
            .await
            .expect("the first insert is the normal case");

        let second = insert(&pool, "doc-1", "second.pdf", Some(9), "pdf", 11).await;
        assert!(
            second.is_err(),
            "a duplicate id must be reported: OR IGNORE would return Ok and the caller \
             would report an upload whose vectors are in Qdrant and whose row is not \
             in SQLite"
        );

        // And the first row is untouched: not overwritten, not duplicated.
        let row = find_by_id(&pool, "doc-1").await.unwrap().unwrap();
        assert_eq!(row.filename, "first.pdf");
        assert_eq!(row.chunk_count, 7);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    /// The ordinary case is unaffected: the same filename may be uploaded twice,
    /// since only `id` is unique.
    #[tokio::test]
    async fn two_documents_may_share_a_filename() {
        let d = tempfile::tempdir().unwrap();
        let pool = migrated(d.path()).await;
        insert(&pool, "doc-1", "report.pdf", Some(1), "pdf", 1)
            .await
            .unwrap();
        insert(&pool, "doc-2", "report.pdf", Some(2), "pdf", 2)
            .await
            .expect("only the id is unique, so a repeated filename is fine");
        assert_eq!(list_active(&pool).await.unwrap().len(), 2);
    }
}
