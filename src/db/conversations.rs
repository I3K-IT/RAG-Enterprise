//! Chat message persistence (`chat_messages` table) e metadata conversazioni (`conversations` table).
//!
//! MAX_MESSAGES_PER_USER = 100 (parità Python).

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;

pub const MAX_MESSAGES_PER_USER: i64 = 100;

// ── Conversation metadata ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ConversationRow {
    pub id: String,
    pub user_id: i64,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn create_conversation(pool: &SqlitePool, user_id: i64) -> Result<ConversationRow> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO conversations (id, user_id, title, created_at, updated_at)
         VALUES (?, ?, 'New Conversation', ?, ?)"
    )
    .bind(&id)
    .bind(user_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(ConversationRow {
        id,
        user_id,
        title: "New Conversation".to_owned(),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub async fn list_conversations(pool: &SqlitePool, user_id: i64) -> Result<Vec<ConversationRow>> {
    let rows = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, user_id, title, created_at, updated_at
         FROM conversations WHERE user_id = ?
         ORDER BY updated_at DESC"
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn rename_conversation(
    pool: &SqlitePool,
    conv_id: &str,
    user_id: i64,
    title: &str,
) -> Result<bool> {
    let now = Utc::now().to_rfc3339();
    let affected = sqlx::query(
        "UPDATE conversations SET title = ?, updated_at = ?
         WHERE id = ? AND user_id = ?"
    )
    .bind(title)
    .bind(now)
    .bind(conv_id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Deletes the conversation and all of its messages (manual cascade).
///
/// SECURITY (IDOR): ownership is verified BEFORE touching any message. An
/// earlier version deleted `chat_messages` by conv_id with no user filter and
/// only filtered by user_id on the conversation itself, so a user could delete
/// someone else's messages by passing a conv_id that was not theirs.
///
/// Now: an atomic transaction where the conversation, filtered by user_id, is
/// removed first. If it does not belong to the user then `rows_affected == 0`,
/// the transaction rolls back and no message is touched. The messages are
/// filtered by user_id as well, as defence in depth.
pub async fn delete_conversation(
    pool: &SqlitePool,
    conv_id: &str,
    user_id: i64,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    let affected = sqlx::query(
        "DELETE FROM conversations WHERE id = ? AND user_id = ?"
    )
    .bind(conv_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if affected == 0 {
        // Not the user's, or does not exist: touch no messages.
        tx.rollback().await?;
        return Ok(false);
    }

    sqlx::query("DELETE FROM chat_messages WHERE conversation_id = ? AND user_id = ?")
        .bind(conv_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(true)
}

/// Updates the conversation's updated_at (called after inserting a message).
pub async fn touch_conversation(pool: &SqlitePool, conv_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(conv_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Chat messages ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChatMessageRow {
    pub id: i64,
    pub user_id: i64,
    pub role: String,
    pub content: String,
    pub sources: Option<String>,
    pub timestamp: String,
    pub conversation_id: Option<String>,
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: i64,
    role: &str,
    content: &str,
    sources: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let id = sqlx::query(
        "INSERT INTO chat_messages (user_id, role, content, sources, timestamp, conversation_id)
         VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(user_id)
    .bind(role)
    .bind(content)
    .bind(sources)
    .bind(now)
    .bind(conversation_id)
    .execute(pool)
    .await?
    .last_insert_rowid();

    if let Some(cid) = conversation_id {
        let _ = touch_conversation(pool, cid).await;
    }

    Ok(id)
}

/// Messages of a specific conversation, ordered ASC (chronologically).
pub async fn list_by_conversation(
    pool: &SqlitePool,
    conv_id: &str,
    user_id: i64,
) -> Result<Vec<ChatMessageRow>> {
    let rows = sqlx::query_as::<_, ChatMessageRow>(
        "SELECT id, user_id, role, content, sources, timestamp, conversation_id
         FROM chat_messages
         WHERE conversation_id = ? AND user_id = ?
         ORDER BY id ASC"
    )
    .bind(conv_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The user's last N messages, used for history injection into the prompt.
/// When conv_id is Some, restricted to that conversation.
pub async fn list_by_user(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<ChatMessageRow>> {
    let rows = sqlx::query_as::<_, ChatMessageRow>(
        "SELECT id, user_id, role, content, sources, timestamp, conversation_id
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

/// The last N messages of a specific conversation, for history injection.
pub async fn list_by_conv_for_history(
    pool: &SqlitePool,
    conv_id: &str,
    user_id: i64,
    limit: i64,
) -> Result<Vec<ChatMessageRow>> {
    let rows = sqlx::query_as::<_, ChatMessageRow>(
        "SELECT id, user_id, role, content, sources, timestamp, conversation_id
         FROM chat_messages
         WHERE conversation_id = ? AND user_id = ?
         ORDER BY id DESC
         LIMIT ?"
    )
    .bind(conv_id)
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_by_user(pool: &SqlitePool, user_id: i64) -> Result<u64> {
    let affected = sqlx::query("DELETE FROM chat_messages WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected)
}
