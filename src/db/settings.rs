//! Runtime settings an administrator changes from the admin panel, one row
//! per key in the `settings` table (migrations/0003_settings.sql).
//!
//! A key with no row means "the default", and the default lives in the code
//! that uses the setting: resetting a setting deletes its row rather than
//! writing the default into it, so a later release that improves a default
//! reaches every install that never changed it.
//!
//! Read on every use, not cached: a lookup by primary key on SQLite costs
//! far less than the request around it, and a change then applies to the
//! very next question with nothing to invalidate.

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

/// The instructions the answering model is given (rag::prompt).
pub const ANSWER_INSTRUCTIONS: &str = "answer_instructions";

/// The stored value of `key`, if one was set.
pub async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let value = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value)
}

/// Stores `value` for `key`, replacing what was there.
pub async fn set(pool: &SqlitePool, key: &str, value: &str, updated_by: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES (?, ?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET
             value = excluded.value,
             updated_at = excluded.updated_at,
             updated_by = excluded.updated_by",
    )
    .bind(key)
    .bind(value)
    .bind(Utc::now().to_rfc3339())
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Removes `key`, so its default applies again. True when a value was there.
pub async fn reset(pool: &SqlitePool, key: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool(path: &std::path::Path) -> SqlitePool {
        let url = format!("sqlite://{}", path.display());
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn a_setting_is_absent_until_set_and_absent_again_after_reset() {
        let d = tempfile::tempdir().unwrap();
        let p = pool(&d.path().join("s.db")).await;
        assert_eq!(get(&p, ANSWER_INSTRUCTIONS).await.unwrap(), None);

        set(&p, ANSWER_INSTRUCTIONS, "Answer briefly.", "admin").await.unwrap();
        assert_eq!(get(&p, ANSWER_INSTRUCTIONS).await.unwrap().as_deref(), Some("Answer briefly."));
        set(&p, ANSWER_INSTRUCTIONS, "Answer at length.", "admin").await.unwrap();
        assert_eq!(get(&p, ANSWER_INSTRUCTIONS).await.unwrap().as_deref(), Some("Answer at length."));

        assert!(reset(&p, ANSWER_INSTRUCTIONS).await.unwrap());
        assert_eq!(get(&p, ANSWER_INSTRUCTIONS).await.unwrap(), None);
        assert!(!reset(&p, ANSWER_INSTRUCTIONS).await.unwrap(), "nothing left to reset");
    }

    /// The value is in the database, not in the process: a new pool on the
    /// same file - what a restart opens - reads it back.
    #[tokio::test]
    async fn a_setting_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("s.db");
        {
            let p = pool(&path).await;
            set(&p, ANSWER_INSTRUCTIONS, "Persisted.", "admin").await.unwrap();
            p.close().await;
        }
        let p = pool(&path).await;
        assert_eq!(get(&p, ANSWER_INSTRUCTIONS).await.unwrap().as_deref(), Some("Persisted."));
        let by: Option<String> = sqlx::query_scalar("SELECT updated_by FROM settings WHERE key = ?")
            .bind(ANSWER_INSTRUCTIONS)
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(by.as_deref(), Some("admin"));
    }
}
