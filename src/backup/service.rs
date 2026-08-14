//! Backup and restore: SQLite + Qdrant snapshot ⇄ tar.gz archive.
//!
//! Backup:
//! 1. SQLite: VACUUM INTO for a consistent copy (WAL-safe)
//! 2. Qdrant: POST /collections/{name}/snapshots → GET snapshot download
//! 3. Pack both into a tar.gz archive under backup_dir
//!
//! Restore is the exact inverse — see `restore_backup`.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use flate2::Compression;
use flate2::write::GzEncoder;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

/// Create a backup archive in `backup_dir`.
/// Returns the path of the `.tar.gz` file created.
pub async fn create_backup(
    db: &SqlitePool,
    _db_path: &str,
    qdrant_url: &str,
    qdrant_collection: &str,
    backup_dir: &str,
) -> Result<PathBuf> {
    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let dir = Path::new(backup_dir);
    std::fs::create_dir_all(dir).with_context(|| format!("create backup dir {}", dir.display()))?;

    let work_dir = dir.join(format!("backup_{ts}"));
    std::fs::create_dir_all(&work_dir)?;

    // 1. SQLite: VACUUM INTO for a WAL-safe consistent snapshot
    let sqlite_dest = work_dir.join("rag_users.db");
    sqlx::query(&format!("VACUUM INTO '{}'", sqlite_dest.display()))
        .execute(db)
        .await
        .with_context(|| format!("VACUUM INTO {}", sqlite_dest.display()))?;
    tracing::info!(path = %sqlite_dest.display(), "SQLite backup done");

    // 2. Qdrant snapshot (best-effort — skip on failure)
    let snap_result = create_qdrant_snapshot(qdrant_url, qdrant_collection, &work_dir).await;
    match snap_result {
        Ok(p) => tracing::info!(path = %p.display(), "Qdrant snapshot saved"),
        Err(e) => tracing::warn!(error = %e, "Qdrant snapshot skipped (non-fatal)"),
    }

    // 3. Pack work_dir into a tar.gz archive
    let archive_path = dir.join(format!("backup_{ts}.tar.gz"));
    pack_tar_gz(&work_dir, &archive_path)?;

    // 4. Remove the temp work directory
    let _ = std::fs::remove_dir_all(&work_dir);

    tracing::info!(archive = %archive_path.display(), "backup complete");
    Ok(archive_path)
}

/// Pack the directory at `src` into a tar.gz at `dest`.
fn pack_tar_gz(src: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::create(dest)
        .with_context(|| format!("create archive {}", dest.display()))?;
    let gz = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all(".", src)
        .with_context(|| format!("tar append {}", src.display()))?;
    tar.finish()?;
    Ok(())
}

// ── Qdrant snapshot ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SnapshotInfo {
    name: String,
}

#[derive(Deserialize)]
struct SnapshotResult {
    result: SnapshotInfo,
}

async fn create_qdrant_snapshot(
    qdrant_url: &str,
    collection: &str,
    dest_dir: &Path,
) -> Result<PathBuf> {
    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    // POST /collections/{name}/snapshots → triggers creation, returns snapshot metadata
    let url = format!("{qdrant_url}/collections/{collection}/snapshots");
    let resp: SnapshotResult = http
        .post(&url)
        .send()
        .await
        .context("qdrant snapshot POST")?
        .json()
        .await
        .context("qdrant snapshot POST response")?;

    let snap_name = resp.result.name;

    // GET /collections/{name}/snapshots/{snap_name} → download binary
    let dl_url = format!("{qdrant_url}/collections/{collection}/snapshots/{snap_name}");
    let bytes = http
        .get(&dl_url)
        .send()
        .await
        .context("qdrant snapshot download")?
        .bytes()
        .await
        .context("qdrant snapshot download bytes")?;

    let out = dest_dir.join(format!("{collection}.snapshot"));
    std::fs::write(&out, &bytes)
        .with_context(|| format!("write snapshot {}", out.display()))?;

    Ok(out)
}

// ── Restore ───────────────────────────────────────────────────────────────────

/// What a restore actually put back. Reported to the caller rather than
/// summarised as a bare "ok": a backup taken before Qdrant was reachable
/// contains no snapshot, and the admin needs to know that only the database
/// came back.
#[derive(Debug, Default, Serialize)]
pub struct RestoreReport {
    pub qdrant_restored: bool,
    pub sqlite_tables: Vec<String>,
    pub sqlite_rows: u64,
}

/// Restore an archive previously produced by `create_backup`.
///
/// `archive_name` is a plain file name inside `backup_dir` — see
/// `resolve_archive`, which is what stops a request from naming a path
/// elsewhere on the host.
///
/// **Order matters.** Qdrant is restored first, the database second. Neither
/// half can be rolled back once the other has been written, so the ordering is
/// chosen to fail on the side that leaves the least damage: the Qdrant upload
/// is the slow, network-facing step and by far the likelier to fail, and if it
/// does we return before touching SQLite, leaving the installation exactly as
/// it was. The reverse order would replace the metadata and then strand it
/// against the old vectors.
pub async fn restore_backup(
    db: &SqlitePool,
    qdrant_url: &str,
    qdrant_collection: &str,
    backup_dir: &str,
    archive_name: &str,
) -> Result<RestoreReport> {
    let archive = resolve_archive(backup_dir, archive_name)?;

    let tmp = tempfile::tempdir().context("temp dir for restore")?;
    unpack_tar_gz(&archive, tmp.path())?;

    let mut report = RestoreReport::default();

    let snapshot = tmp.path().join(format!("{qdrant_collection}.snapshot"));
    if snapshot.is_file() {
        upload_qdrant_snapshot(qdrant_url, qdrant_collection, &snapshot)
            .await
            .context("restoring the Qdrant snapshot (nothing was changed)")?;
        report.qdrant_restored = true;
        tracing::info!(collection = qdrant_collection, "Qdrant snapshot restored");
    } else {
        tracing::warn!(
            archive = archive_name,
            "the archive holds no Qdrant snapshot: restoring the database only"
        );
    }

    let sqlite = tmp.path().join("rag_users.db");
    if sqlite.is_file() {
        let (tables, rows) = restore_sqlite(db, &sqlite).await?;
        tracing::info!(tables = tables.len(), rows, "SQLite restored");
        report.sqlite_tables = tables;
        report.sqlite_rows = rows;
    }

    Ok(report)
}

/// Resolve `name` to an archive inside `backup_dir`.
///
/// The name arrives in an HTTP body, so it is treated as hostile: anything
/// carrying a separator or a parent component could point the unpacker at an
/// arbitrary file on the host.
fn resolve_archive(backup_dir: &str, name: &str) -> Result<PathBuf> {
    let is_plain_file_name =
        Path::new(name).components().collect::<Vec<_>>().as_slice() == [Component::Normal(name.as_ref())];
    if !is_plain_file_name {
        anyhow::bail!("invalid archive name: {name:?}");
    }
    if !name.ends_with(".tar.gz") {
        anyhow::bail!("not a backup archive: {name:?}");
    }

    let path = Path::new(backup_dir).join(name);
    if !path.is_file() {
        anyhow::bail!("archive not found: {name}");
    }
    Ok(path)
}

/// Unpack `archive` into `dest`, refusing entries that would escape it.
///
/// A tar entry may name any path it likes, including `../../etc/passwd`
/// ("tar slip"). Our own archives never do, but the file on disk is not
/// necessarily one of ours.
fn unpack_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("opening archive {}", archive.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);

    for entry in tar.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading tar entry")?;
        let path = entry.path().context("tar entry path")?.into_owned();

        if !is_safe_entry_path(&path) {
            anyhow::bail!("archive entry escapes the destination: {}", path.display());
        }

        let out = dest.join(&path);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        entry
            .unpack(&out)
            .with_context(|| format!("extracting {}", path.display()))?;
    }
    Ok(())
}

/// An entry path is safe when it is relative and walks only downwards.
fn is_safe_entry_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

async fn upload_qdrant_snapshot(qdrant_url: &str, collection: &str, snapshot: &Path) -> Result<()> {
    let bytes = std::fs::read(snapshot)
        .with_context(|| format!("reading snapshot {}", snapshot.display()))?;

    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    // priority=snapshot: the snapshot's contents win over whatever the
    // collection holds now. That is the whole point of a restore — the default
    // priority would keep newer local records and silently give back a mixture.
    let url = format!("{qdrant_url}/collections/{collection}/snapshots/upload?priority=snapshot");
    let form = reqwest::multipart::Form::new().part(
        "snapshot",
        reqwest::multipart::Part::bytes(bytes).file_name("restore.snapshot"),
    );

    let resp = http.post(&url).multipart(form).send().await.context("qdrant snapshot upload")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("qdrant refused the snapshot ({status}): {body}");
    }
    Ok(())
}

/// Copy every table of the backup over the live database, in one transaction.
///
/// Returns the tables restored and the total rows written.
async fn restore_sqlite(db: &SqlitePool, src: &Path) -> Result<(Vec<String>, u64)> {
    // One connection for the whole operation: ATTACH and the foreign_keys
    // pragma are per-connection state, and a pool hands out a different
    // connection per statement.
    let mut conn = db.acquire().await.context("acquiring a connection for the restore")?;

    let src_path = src.to_string_lossy().replace('\'', "''");

    // foreign_keys can only be switched outside a transaction, and it has to be
    // off: the copy empties `users` before refilling it, which every table
    // referencing it would otherwise reject.
    sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *conn).await?;
    sqlx::query(&format!("ATTACH DATABASE '{src_path}' AS backup"))
        .execute(&mut *conn)
        .await
        .context("attaching the backup database")?;

    let result = copy_tables(&mut conn).await;

    // Restore connection state whatever happened, so the connection is safe to
    // hand back to the pool.
    let _ = sqlx::query("DETACH DATABASE backup").execute(&mut *conn).await;
    let _ = sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await;

    result
}

async fn copy_tables(conn: &mut SqliteConnection) -> Result<(Vec<String>, u64)> {
    // _sqlx_migrations is deliberately excluded: the schema belongs to the
    // binary that is running, not to the archive. Copying it back would tell a
    // newer binary that migrations it has already applied are still pending.
    let candidates: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM backup.sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(&mut *conn)
    .await
    .context("listing the tables in the backup")?;

    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

    let mut restored = Vec::new();
    let mut total_rows: u64 = 0;

    for table in candidates {
        let live_cols = column_names(conn, "main", &table).await?;
        if live_cols.is_empty() {
            // Present in the archive, absent from this schema. Skipping it is
            // what lets an older backup restore into a newer binary.
            tracing::warn!(table, "table not in the current schema, skipped");
            continue;
        }
        let backup_cols: HashSet<String> =
            column_names(conn, "backup", &table).await?.into_iter().collect();

        // Intersection in the live schema's order: a column added by a later
        // migration simply keeps its default.
        let shared: Vec<String> = live_cols.into_iter().filter(|c| backup_cols.contains(c)).collect();
        if shared.is_empty() {
            tracing::warn!(table, "no column in common with the backup, skipped");
            continue;
        }

        let t = quote_ident(&table);
        let cols = shared.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");

        sqlx::query(&format!("DELETE FROM main.{t}"))
            .execute(&mut *conn)
            .await
            .with_context(|| format!("emptying {table}"))?;
        let done = sqlx::query(&format!(
            "INSERT INTO main.{t} ({cols}) SELECT {cols} FROM backup.{t}"
        ))
        .execute(&mut *conn)
        .await
        .with_context(|| format!("copying {table}"))?;

        total_rows += done.rows_affected();
        restored.push(table);
    }

    sqlx::query("COMMIT").execute(&mut *conn).await.context("committing the restore")?;
    Ok((restored, total_rows))
}

async fn column_names(conn: &mut SqliteConnection, schema: &str, table: &str) -> Result<Vec<String>> {
    // PRAGMA takes no bind parameters, so the identifiers are interpolated.
    // `schema` is ours; `table` comes from the archive and is quoted. It can
    // only ever match a table that also exists in the live schema, which is the
    // check that actually matters.
    let rows = sqlx::query(&format!("PRAGMA {schema}.table_info({})", quote_ident(table)))
        .fetch_all(&mut *conn)
        .await
        .with_context(|| format!("reading the columns of {schema}.{table}"))?;
    Ok(rows.iter().map(|r| r.get::<String, _>("name")).collect())
}

/// Quote an SQL identifier, doubling any embedded quote.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// List backup archives in the backup directory (*.tar.gz), sorted newest first.
pub fn list_backups(backup_dir: &str) -> Vec<String> {
    let dir = Path::new(backup_dir);
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut names: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tar.gz"))
        .collect();
    names.sort_by(|a, b| b.cmp(a)); // newest first (lexicographic on timestamp)
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── archive name validation ───────────────────────────────────────────────
    // The name arrives in an HTTP body. These are the cases that would let a
    // caller point the unpacker outside the backup directory.

    fn dir_with(name: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(name), b"not really an archive").unwrap();
        d
    }

    #[test]
    fn resolve_archive_accepts_a_plain_name_that_exists() {
        let d = dir_with("backup_20260101_000000.tar.gz");
        let got = resolve_archive(d.path().to_str().unwrap(), "backup_20260101_000000.tar.gz");
        assert_eq!(got.unwrap(), d.path().join("backup_20260101_000000.tar.gz"));
    }

    #[test]
    fn resolve_archive_rejects_traversal_and_absolute_paths() {
        let d = dir_with("ok.tar.gz");
        let dir = d.path().to_str().unwrap();
        for hostile in [
            "../ok.tar.gz",
            "../../etc/passwd.tar.gz",
            "sub/ok.tar.gz",
            "/etc/passwd.tar.gz",
            "./ok.tar.gz",
            "",
        ] {
            assert!(
                resolve_archive(dir, hostile).is_err(),
                "resolve_archive must reject {hostile:?}"
            );
        }
    }

    #[test]
    fn resolve_archive_rejects_a_name_that_is_not_an_archive() {
        let d = dir_with("rag_users.db");
        assert!(resolve_archive(d.path().to_str().unwrap(), "rag_users.db").is_err());
    }

    #[test]
    fn resolve_archive_rejects_a_missing_file() {
        let d = tempfile::tempdir().unwrap();
        assert!(resolve_archive(d.path().to_str().unwrap(), "absent.tar.gz").is_err());
    }

    // ── tar entry paths ───────────────────────────────────────────────────────

    #[test]
    fn entry_paths_walking_outside_the_destination_are_refused() {
        assert!(is_safe_entry_path(Path::new("rag_users.db")));
        assert!(is_safe_entry_path(Path::new("./rag_docs.snapshot")));
        assert!(is_safe_entry_path(Path::new("nested/file")));

        assert!(!is_safe_entry_path(Path::new("../escape")));
        assert!(!is_safe_entry_path(Path::new("a/../../escape")));
        assert!(!is_safe_entry_path(Path::new("/etc/passwd")));
    }

    #[test]
    fn identifiers_are_quoted_and_embedded_quotes_doubled() {
        assert_eq!(quote_ident("users"), "\"users\"");
        // A table name from a hostile archive must not be able to close the
        // quoting and continue the statement.
        assert_eq!(quote_ident("a\"; DROP TABLE users --"), "\"a\"\"; DROP TABLE users --\"");
    }

    // ── SQLite restore, end to end ────────────────────────────────────────────

    async fn pool_at(path: &Path) -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        SqlitePool::connect_with(opts).await.unwrap()
    }

    #[tokio::test]
    async fn restore_replaces_live_rows_with_the_archived_ones() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;

        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(&live).await.unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'archived')")
            .execute(&live).await.unwrap();

        // Take the backup exactly as create_backup does.
        let backup_file = d.path().join("backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", backup_file.display()))
            .execute(&live).await.unwrap();

        // Drift away from it: one row changed, one row added.
        sqlx::query("UPDATE users SET name = 'live' WHERE id = 1").execute(&live).await.unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (2, 'added later')")
            .execute(&live).await.unwrap();

        let (tables, rows) = restore_sqlite(&live, &backup_file).await.unwrap();
        assert_eq!(tables, vec!["users".to_string()]);
        assert_eq!(rows, 1);

        // Replaced, not merged: the row added after the backup is gone.
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM users ORDER BY id")
            .fetch_all(&live).await.unwrap();
        assert_eq!(names, vec!["archived".to_string()]);
    }

    #[tokio::test]
    async fn a_column_added_after_the_backup_survives_the_restore() {
        // An older archive must still restore into a newer schema: the columns
        // it does not know about keep their default rather than blocking it.
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;

        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(&live).await.unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'old')")
            .execute(&live).await.unwrap();
        let backup_file = d.path().join("backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", backup_file.display()))
            .execute(&live).await.unwrap();

        sqlx::query("ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user'")
            .execute(&live).await.unwrap();

        let (tables, rows) = restore_sqlite(&live, &backup_file).await.unwrap();
        assert_eq!(tables, vec!["users".to_string()]);
        assert_eq!(rows, 1);

        let (name, role): (String, String) =
            sqlx::query_as("SELECT name, role FROM users WHERE id = 1")
                .fetch_one(&live).await.unwrap();
        assert_eq!(name, "old");
        assert_eq!(role, "user");
    }

    #[tokio::test]
    async fn a_table_dropped_from_the_schema_is_skipped_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;

        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)").execute(&live).await.unwrap();
        sqlx::query("CREATE TABLE legacy (id INTEGER PRIMARY KEY)").execute(&live).await.unwrap();
        sqlx::query("INSERT INTO users (id) VALUES (1)").execute(&live).await.unwrap();
        sqlx::query("INSERT INTO legacy (id) VALUES (9)").execute(&live).await.unwrap();
        let backup_file = d.path().join("backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", backup_file.display()))
            .execute(&live).await.unwrap();

        sqlx::query("DROP TABLE legacy").execute(&live).await.unwrap();

        let (tables, _) = restore_sqlite(&live, &backup_file).await.unwrap();
        assert_eq!(tables, vec!["users".to_string()], "legacy must be skipped, not restored");
    }

    #[tokio::test]
    async fn the_migration_table_is_never_copied_back() {
        // The schema belongs to the running binary. Restoring _sqlx_migrations
        // would tell it that migrations it has already applied are pending.
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;

        sqlx::query("CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY)")
            .execute(&live).await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)").execute(&live).await.unwrap();
        sqlx::query("INSERT INTO _sqlx_migrations (version) VALUES (1)")
            .execute(&live).await.unwrap();
        let backup_file = d.path().join("backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", backup_file.display()))
            .execute(&live).await.unwrap();

        sqlx::query("INSERT INTO _sqlx_migrations (version) VALUES (2)")
            .execute(&live).await.unwrap();

        let (tables, _) = restore_sqlite(&live, &backup_file).await.unwrap();
        assert!(!tables.contains(&"_sqlx_migrations".to_string()));

        let versions: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&live).await.unwrap();
        assert_eq!(versions, vec![1, 2], "the applied migrations must be left alone");
    }

    #[tokio::test]
    async fn foreign_keys_do_not_block_the_restore_and_are_on_again_afterwards() {
        // The copy empties `users` before refilling it, which a table
        // referencing it would reject while enforcement is on.
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;

        sqlx::query("PRAGMA foreign_keys = ON").execute(&live).await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)").execute(&live).await.unwrap();
        sqlx::query(
            "CREATE TABLE chat_messages (id INTEGER PRIMARY KEY, user_id INTEGER, \
             FOREIGN KEY (user_id) REFERENCES users(id))",
        ).execute(&live).await.unwrap();
        sqlx::query("INSERT INTO users (id) VALUES (1)").execute(&live).await.unwrap();
        sqlx::query("INSERT INTO chat_messages (id, user_id) VALUES (1, 1)")
            .execute(&live).await.unwrap();

        let backup_file = d.path().join("backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", backup_file.display()))
            .execute(&live).await.unwrap();

        let (mut tables, rows) = restore_sqlite(&live, &backup_file).await.unwrap();
        tables.sort();
        assert_eq!(tables, vec!["chat_messages".to_string(), "users".to_string()]);
        assert_eq!(rows, 2);

        let on: i64 = sqlx::query_scalar("PRAGMA foreign_keys").fetch_one(&live).await.unwrap();
        assert_eq!(on, 1, "enforcement must be back on when the restore returns");
    }
}
