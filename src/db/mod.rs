pub mod conversations;
pub mod documents;
pub mod settings;
pub mod users;

use anyhow::Result;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

pub async fn connect(url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        // Stated rather than assumed. sqlx already turns foreign_keys ON for
        // every SQLite connection it opens (sqlx-sqlite 0.8, options/mod.rs)
        // — unlike SQLite itself, which leaves it off for backward
        // compatibility — and delete_conversation in db::conversations
        // depends on that being true: it deletes children before parent
        // precisely because the constraint IS enforced. That is a load-
        // bearing invariant of this schema, so it belongs in our own code
        // where it can be read, not in a default we happen to inherit.
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    let pool = SqlitePool::connect_with(opts).await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
