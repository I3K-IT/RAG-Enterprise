//! Chat message persistence (`chat_messages` table) and conversation metadata (`conversations` table).
//!
//! MAX_MESSAGES_PER_USER = 100.

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
/// Children before parent: `chat_messages.conversation_id` has a (non-
/// cascading) `FOREIGN KEY ... REFERENCES conversations(id)`
/// (migrations/0002_conversations.sql) and sqlx enables `PRAGMA
/// foreign_keys = ON` by default — deleting the conversation row first, as
/// an earlier version of this function did, unconditionally fails with
/// SQLite error 787 the moment the conversation has any message at all
/// (i.e. every real conversation a user would actually want to delete).
///
/// SECURITY (IDOR): both statements independently filter by `user_id`, not
/// just `conv_id` — so this user's DELETE can only ever touch THIS user's
/// own rows in either table, regardless of which order they run in. (An
/// even earlier version deleted `chat_messages` by conv_id alone, with no
/// user filter there, relying solely on checking the conversation's
/// ownership first — that is what made the delete order load-bearing for
/// security. It no longer is, now that both statements carry their own
/// filter, but the double-check stays as defence in depth.)
pub async fn delete_conversation(
    pool: &SqlitePool,
    conv_id: &str,
    user_id: i64,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM chat_messages WHERE conversation_id = ? AND user_id = ?")
        .bind(conv_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    let affected = sqlx::query(
        "DELETE FROM conversations WHERE id = ? AND user_id = ?"
    )
    .bind(conv_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    tx.commit().await?;
    Ok(affected > 0)
}

/// Whether this conversation exists AND belongs to this user.
///
/// The `conversation_id` on a query request is a plain string chosen by the
/// client, not something the server handed out and can trust. Every read here
/// filters by `user_id` as well, so a foreign id has never been able to
/// disclose anything — but the write paths took it at face value, which let a
/// user file their own messages under someone else's conversation and, through
/// touch_conversation, move that conversation to the top of its owner's list.
/// Callers that accept the id from a request check it here first and answer
/// 404 — not 403 — when it comes back false: whether a conversation belonging
/// to someone else exists is itself not this user's business.
pub async fn is_owned_by(pool: &SqlitePool, conv_id: &str, user_id: i64) -> Result<bool> {
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM conversations WHERE id = ? AND user_id = ?")
            .bind(conv_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

/// Updates the conversation's updated_at (called after inserting a message).
///
/// Filtered by `user_id` for the same reason every other statement in this
/// module is: so that reaching it with a conversation id belonging to someone
/// else is a no-op rather than a way to reorder their conversation list.
pub async fn touch_conversation(pool: &SqlitePool, conv_id: &str, user_id: i64) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ? AND user_id = ?")
        .bind(now)
        .bind(conv_id)
        .bind(user_id)
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
        let _ = touch_conversation(pool, cid, user_id).await;
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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A real file-backed pool through crate::db::connect, so these tests see
    /// the same PRAGMAs and the same migrated schema production does — an
    /// in-memory pool built by hand would prove nothing about either.
    async fn pool(dir: &std::path::Path) -> SqlitePool {
        let url = format!("sqlite://{}", dir.join("test.db").display());
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, role, created_at)
             VALUES (1, 'alice', 'alice@example.com', 'x', 'user', '2026-01-01T00:00:00Z'),
                    (2, 'bob',   'bob@example.com',   'x', 'user', '2026-01-01T00:00:00Z')",
        )
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let d = tempfile::tempdir().unwrap();
        let p = pool(d.path()).await;
        let on: i64 = sqlx::query_scalar("PRAGMA foreign_keys").fetch_one(&p).await.unwrap();
        assert_eq!(on, 1, "delete_conversation's child-before-parent order depends on this");
    }

    #[tokio::test]
    async fn ownership_is_checked_against_the_user_not_just_the_id() {
        let d = tempfile::tempdir().unwrap();
        let p = pool(d.path()).await;
        let conv = create_conversation(&p, 1).await.unwrap();

        assert!(is_owned_by(&p, &conv.id, 1).await.unwrap());
        // The id is real and the user is authenticated — it is still not
        // theirs, which is the whole case this guards.
        assert!(!is_owned_by(&p, &conv.id, 2).await.unwrap());
        assert!(!is_owned_by(&p, "no-such-conversation", 1).await.unwrap());
    }

    #[tokio::test]
    async fn touching_someone_elses_conversation_does_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = pool(d.path()).await;
        let conv = create_conversation(&p, 1).await.unwrap();

        touch_conversation(&p, &conv.id, 2).await.unwrap();

        let after: String =
            sqlx::query_scalar("SELECT updated_at FROM conversations WHERE id = ?")
                .bind(&conv.id)
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(
            after, conv.updated_at,
            "bob must not be able to move alice's conversation to the top of her list"
        );
    }
}
