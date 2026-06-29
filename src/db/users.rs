//! User persistence (rag_users.db → `users` table).
//!
//! Schema mirrors MAPPA §4 exactly:
//! id, username, email, password_hash, role, created_at, last_login, is_active

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

use crate::auth::rbac::Role;

#[derive(Debug, sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: String,
    pub last_login: Option<String>,
    pub is_active: i64,
}

impl UserRow {
    pub fn role(&self) -> Role {
        self.role.parse().unwrap_or(Role::User)
    }
}

pub async fn find_by_username(pool: &SqlitePool, username: &str) -> Result<Option<UserRow>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, email, password_hash, role, created_at, last_login, is_active
         FROM users WHERE username = ? AND is_active = 1"
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn create(
    pool: &SqlitePool,
    username: &str,
    email: &str,
    password_hash: &str,
    role: Role,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let role_str = role.to_string();
    let id = sqlx::query(
        "INSERT INTO users (username, email, password_hash, role, created_at, is_active)
         VALUES (?, ?, ?, ?, ?, 1)"
    )
    .bind(username)
    .bind(email)
    .bind(password_hash)
    .bind(role_str)
    .bind(now)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn touch_last_login(pool: &SqlitePool, user_id: i64) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE users SET last_login = ? WHERE id = ?")
        .bind(now)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
