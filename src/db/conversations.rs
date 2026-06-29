//! Chat message persistence (rag_users.db → `chat_messages` table).
//!
//! Schema mirrors MAPPA §4: id, user_id, role, content, sources (JSON), timestamp.
//! MAX_MESSAGES_PER_USER = 100 (same as Python).

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

pub const MAX_MESSAGES_PER_USER: i64 = 100;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChatMessageRow {
    pub id: i64,
    pub user_id: i64,
    pub role: String,
    pub content: String,
    pub sources: Option<String>,
    pub timestamp: String,
}

pub async fn list_by_user(pool: &SqlitePool, user_id: i64, limit: i64) -> Result<Vec<ChatMessageRow>> {
    let rows = sqlx::query_as::<_, ChatMessageRow>(
        "SELECT id, user_id, role, content, sources, timestamp
         FROM chat_messages
         WHERE user_id = ?
         ORDER BY timestamp DESC
         LIMIT ?"
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: i64,
    role: &str,
    content: &str,
    sources: Option<&str>,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let id = sqlx::query(
        "INSERT INTO chat_messages (user_id, role, content, sources, timestamp)
         VALUES (?, ?, ?, ?, ?)"
    )
    .bind(user_id)
    .bind(role)
    .bind(content)
    .bind(sources)
    .bind(now)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn delete_by_user(pool: &SqlitePool, user_id: i64) -> Result<u64> {
    let affected = sqlx::query("DELETE FROM chat_messages WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected)
}
