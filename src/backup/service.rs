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

    // 1. SQLite: VACUUM INTO for a WAL-safe consistent snapshot, then ask
    //    SQLite whether what came out is actually a sound database.
    let sqlite_dest = work_dir.join("rag_users.db");
    sqlx::query(&format!("VACUUM INTO '{}'", sqlite_dest.display()))
        .execute(db)
        .await
        .with_context(|| format!("VACUUM INTO {}", sqlite_dest.display()))?;
    verify_sqlite_copy(&sqlite_dest).await?;
    let sqlite_digest = digest_of(&sqlite_dest)?;
    tracing::info!(path = %sqlite_dest.display(), size = sqlite_digest.size, "SQLite backup verified");

    // 2. Qdrant snapshot, verified against the size and sha256 Qdrant itself
    //    reports.
    //
    //    Unreachable and corrupt are treated differently on purpose. If Qdrant
    //    does not answer we still write the archive, recording in the manifest
    //    that it has no vectors — losing today's copy of the users and the
    //    document metadata as well would be the worse outcome, and the gap is
    //    stated rather than hidden. But if Qdrant answers with something that
    //    does not match what it says it made, the backup fails: a verifiably
    //    broken archive must never reach the backup directory, because its only
    //    purpose is to look reassuring in a listing until the day it is needed.
    let qdrant_digest = match create_qdrant_snapshot(qdrant_url, qdrant_collection, &work_dir).await
    {
        Ok(digest) => Some(digest),
        Err(e) if is_qdrant_unreachable(&e) => {
            tracing::error!(
                error = %e,
                "Qdrant did not answer: writing a database-only backup. It will NOT restore \
                 your documents — take another one once Qdrant is back."
            );
            let _ = std::fs::remove_file(work_dir.join(format!("{qdrant_collection}.snapshot")));
            None
        }
        Err(e) => return Err(e).context("the Qdrant snapshot could not be verified, backup aborted"),
    };

    // 3. Write the manifest that lets a restore check all of this before
    //    trusting any of it.
    let manifest = BackupManifest {
        format: MANIFEST_FORMAT,
        created: Utc::now().to_rfc3339(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        sqlite: sqlite_digest,
        qdrant_collection: qdrant_digest.as_ref().map(|_| qdrant_collection.to_owned()),
        qdrant: qdrant_digest,
    };
    std::fs::write(
        work_dir.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest).context("serialising backup.json")?,
    )
    .context("writing backup.json")?;

    // 4. Pack work_dir into a tar.gz archive
    let archive_path = dir.join(format!("backup_{ts}.tar.gz"));
    pack_tar_gz(&work_dir, &archive_path)?;

    // 5. Remove the temp work directory
    let _ = std::fs::remove_dir_all(&work_dir);

    tracing::info!(
        archive = %archive_path.display(),
        with_vectors = manifest.qdrant.is_some(),
        "backup complete"
    );
    Ok(archive_path)
}

/// Did the snapshot fail because Qdrant is not there, rather than because what
/// it produced is unusable? Only the first is survivable.
fn is_qdrant_unreachable(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|r| r.is_connect() || r.is_timeout() || r.is_request())
    })
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
    /// Byte length of the snapshot as Qdrant wrote it.
    size: u64,
    /// sha256 of the snapshot file. Present since Qdrant 1.x and verified
    /// against the real thing (1.18.2) — it is the digest of exactly the bytes
    /// the download returns.
    checksum: Option<String>,
}

#[derive(Deserialize)]
struct SnapshotResult {
    result: SnapshotInfo,
}

/// Ask Qdrant for a snapshot, download it, and prove the copy on our disk is
/// the one Qdrant made.
///
/// The download is a plain HTTP body: a truncated response, a proxy that gives
/// up halfway, a full disk — none of those raise an error here, they just leave
/// a shorter file. Writing it unchecked produces an archive that looks fine for
/// months and fails at the only moment it is ever needed. Qdrant reports the
/// size and the sha256 when it creates the snapshot, so there is no reason to
/// take the file on trust.
async fn create_qdrant_snapshot(
    qdrant_url: &str,
    collection: &str,
    dest_dir: &Path,
) -> Result<MemberDigest> {
    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    // POST /collections/{name}/snapshots → triggers creation, returns metadata
    let url = format!("{qdrant_url}/collections/{collection}/snapshots");
    let resp: SnapshotResult = http
        .post(&url)
        .send()
        .await
        .context("qdrant snapshot POST")?
        .json()
        .await
        .context("qdrant snapshot POST response")?;

    let SnapshotInfo { name: snap_name, size: expected_size, checksum: expected_sha } = resp.result;

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

    let digest = digest_of(&out)?;

    if digest.size != expected_size {
        anyhow::bail!(
            "the Qdrant snapshot arrived incomplete: Qdrant wrote {expected_size} bytes, \
             {} reached us",
            digest.size
        );
    }
    match expected_sha {
        Some(expected) if !expected.eq_ignore_ascii_case(&digest.sha256) => anyhow::bail!(
            "the Qdrant snapshot is corrupt: expected sha256 {expected}, got {}",
            digest.sha256
        ),
        Some(_) => tracing::info!(size = digest.size, "Qdrant snapshot verified against its sha256"),
        // Older Qdrant builds report no checksum. The length still catches a
        // truncated download, which is the failure that actually happens.
        None => tracing::warn!(
            size = digest.size,
            "this Qdrant reports no snapshot checksum: only the length could be verified"
        ),
    }

    Ok(digest)
}

// ── archive manifest ──────────────────────────────────────────────────────────

/// One file inside the archive, with the digest that proves it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberDigest {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

/// `backup.json`, written inside every archive so it can be checked before any
/// of it is trusted. Without it a restore has no way to tell a good archive
/// from a half-written one until it is already applying it.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Bumped only if the layout changes in a way an older engine cannot read.
    pub format: u32,
    pub created: String,
    pub engine_version: String,
    pub sqlite: MemberDigest,
    /// Absent when Qdrant could not be reached — never when it answered with
    /// something unusable, which fails the backup outright.
    pub qdrant: Option<MemberDigest>,
    pub qdrant_collection: Option<String>,
}

const MANIFEST_FILE: &str = "backup.json";
const MANIFEST_FORMAT: u32 = 1;

fn digest_of(path: &Path) -> Result<MemberDigest> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening {} to hash it", path.display()))?;
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut file, &mut hasher)
        .with_context(|| format!("reading {} to hash it", path.display()))?;

    Ok(MemberDigest {
        file: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        sha256: format!("{:x}", hasher.finalize()),
        size,
    })
}

/// Ask SQLite whether the copy it just produced is sound.
///
/// VACUUM INTO returning Ok means the statement ran, not that the file on the
/// other side is a healthy database — a disk that fills up mid-write, or fails
/// silently, still leaves a file behind.
async fn verify_sqlite_copy(path: &Path) -> Result<()> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false);
    let pool = SqlitePool::connect_with(opts)
        .await
        .with_context(|| format!("reopening the SQLite copy {}", path.display()))?;

    let verdict: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .context("PRAGMA integrity_check on the SQLite copy")?;
    pool.close().await;

    if verdict != "ok" {
        anyhow::bail!("the SQLite copy did not pass integrity_check: {verdict}");
    }
    Ok(())
}

/// Check every member of an unpacked archive against `backup.json`.
///
/// Returns the manifest, or None for archives written before 0.1.27, which
/// carry no manifest and therefore cannot be checked.
fn verify_unpacked(dir: &Path) -> Result<Option<BackupManifest>> {
    let manifest_path = dir.join(MANIFEST_FILE);
    if !manifest_path.is_file() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&manifest_path).context("reading backup.json")?;
    let manifest: BackupManifest = serde_json::from_str(&raw).context("parsing backup.json")?;

    if manifest.format > MANIFEST_FORMAT {
        anyhow::bail!(
            "this archive is in backup format {} and was written by a newer engine ({}); \
             this one understands up to format {MANIFEST_FORMAT}",
            manifest.format,
            manifest.engine_version
        );
    }

    for member in std::iter::once(&manifest.sqlite).chain(manifest.qdrant.iter()) {
        let path = dir.join(&member.file);
        if !path.is_file() {
            anyhow::bail!("the archive promises {} but does not contain it", member.file);
        }
        let actual = digest_of(&path)?;
        if actual.size != member.size || !actual.sha256.eq_ignore_ascii_case(&member.sha256) {
            anyhow::bail!(
                "{} does not match the archive's own manifest — the backup is damaged \
                 and nothing has been restored (expected sha256 {} at {} bytes, found {} at {})",
                member.file,
                member.sha256,
                member.size,
                actual.sha256,
                actual.size
            );
        }
    }

    Ok(Some(manifest))
}

// ── Restore ───────────────────────────────────────────────────────────────────

/// What a restore actually put back. Reported to the caller rather than
/// summarised as a bare "ok": a backup taken before Qdrant was reachable
/// contains no snapshot, and the admin needs to know that only the database
/// came back.
#[derive(Debug, Default, Serialize)]
pub struct RestoreReport {
    /// Whether the archive could be checked against its own manifest before
    /// being applied. False only for archives written before 0.1.27.
    pub verified: bool,
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

    // Everything is checked against the archive's own manifest BEFORE anything
    // is written. A damaged archive stops here, with the installation
    // untouched, instead of being discovered halfway through the restore.
    let mut report = RestoreReport::default();
    match verify_unpacked(tmp.path())? {
        Some(manifest) => {
            tracing::info!(
                created = %manifest.created,
                engine = %manifest.engine_version,
                "archive verified against its manifest"
            );
            report.verified = true;
            if manifest.qdrant.is_none() {
                tracing::warn!(
                    "this archive was taken while Qdrant was unreachable and holds no vectors: \
                     the documents will not come back, only the database"
                );
            }
        }
        None => tracing::warn!(
            archive = archive_name,
            "archive written before 0.1.27: it carries no manifest, so its contents cannot be \
             verified before being restored"
        ),
    }

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

    // ── archive verification ──────────────────────────────────────────────────

    fn write_manifest(dir: &Path, m: &BackupManifest) {
        std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec_pretty(m).unwrap()).unwrap();
    }

    fn manifest_for(dir: &Path, sqlite: &str, qdrant: Option<&str>) -> BackupManifest {
        BackupManifest {
            format: MANIFEST_FORMAT,
            created: "2026-08-14T00:00:00Z".into(),
            engine_version: "test".into(),
            sqlite: digest_of(&dir.join(sqlite)).unwrap(),
            qdrant: qdrant.map(|q| digest_of(&dir.join(q)).unwrap()),
            qdrant_collection: qdrant.map(|_| "rag_documents".to_string()),
        }
    }

    #[test]
    fn digest_of_hashes_the_file_contents() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x");
        std::fs::write(&f, b"abc").unwrap();
        let got = digest_of(&f).unwrap();
        // sha256("abc")
        assert_eq!(got.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(got.size, 3);
        assert_eq!(got.file, "x");
    }

    #[test]
    fn an_intact_archive_verifies() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rag_users.db"), b"database").unwrap();
        std::fs::write(d.path().join("rag_documents.snapshot"), b"vectors").unwrap();
        write_manifest(d.path(), &manifest_for(d.path(), "rag_users.db", Some("rag_documents.snapshot")));

        let m = verify_unpacked(d.path()).unwrap().expect("manifest present");
        assert!(m.qdrant.is_some());
    }

    #[test]
    fn a_member_that_does_not_match_its_digest_stops_the_restore() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rag_users.db"), b"database").unwrap();
        write_manifest(d.path(), &manifest_for(d.path(), "rag_users.db", None));

        // A single flipped byte — bit rot, a truncated copy, a bad disk.
        std::fs::write(d.path().join("rag_users.db"), b"databasX").unwrap();

        let err = verify_unpacked(d.path()).unwrap_err().to_string();
        assert!(err.contains("does not match"), "unexpected error: {err}");
        assert!(err.contains("nothing has been restored"), "the error must say nothing was applied: {err}");
    }

    #[test]
    fn a_truncated_member_is_caught_even_at_the_same_prefix() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rag_users.db"), b"database-complete").unwrap();
        write_manifest(d.path(), &manifest_for(d.path(), "rag_users.db", None));
        std::fs::write(d.path().join("rag_users.db"), b"database").unwrap();

        assert!(verify_unpacked(d.path()).is_err());
    }

    #[test]
    fn a_member_promised_but_absent_stops_the_restore() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rag_users.db"), b"database").unwrap();
        std::fs::write(d.path().join("rag_documents.snapshot"), b"vectors").unwrap();
        let m = manifest_for(d.path(), "rag_users.db", Some("rag_documents.snapshot"));
        write_manifest(d.path(), &m);
        std::fs::remove_file(d.path().join("rag_documents.snapshot")).unwrap();

        let err = verify_unpacked(d.path()).unwrap_err().to_string();
        assert!(err.contains("does not contain it"), "unexpected error: {err}");
    }

    #[test]
    fn an_archive_from_a_newer_engine_is_refused_rather_than_guessed_at() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rag_users.db"), b"database").unwrap();
        let mut m = manifest_for(d.path(), "rag_users.db", None);
        m.format = MANIFEST_FORMAT + 1;
        m.engine_version = "9.9.9".into();
        write_manifest(d.path(), &m);

        let err = verify_unpacked(d.path()).unwrap_err().to_string();
        assert!(err.contains("newer engine"), "unexpected error: {err}");
    }

    #[test]
    fn an_archive_without_a_manifest_still_restores_but_reports_it_cannot_be_checked() {
        // Archives written before 0.1.27 carry no manifest. They must keep
        // working — refusing them would strand every backup taken so far.
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("rag_users.db"), b"database").unwrap();
        assert!(verify_unpacked(d.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_damaged_sqlite_copy_fails_the_backup_instead_of_being_archived() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("broken.db");
        // A valid header followed by rubbish: the file opens, the database is
        // not sound. This is what a disk filling up mid-write leaves behind.
        let mut bytes = b"SQLite format 3\0".to_vec();
        bytes.extend(vec![0xAB_u8; 4096]);
        std::fs::write(&f, &bytes).unwrap();

        assert!(verify_sqlite_copy(&f).await.is_err());
    }

    // ── against a real Qdrant ─────────────────────────────────────────────────

    /// The snapshot upload is the one step nothing else exercises: it is a live
    /// round trip through Qdrant's own snapshot machinery, and faking it would
    /// test nothing worth testing.
    ///
    ///   QDRANT_URL_FOR_TEST=http://localhost:6333 \
    ///     cargo test qdrant_round_trip -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a running Qdrant — set QDRANT_URL_FOR_TEST"]
    async fn qdrant_round_trip_puts_the_archived_points_back() {
        let Ok(url) = std::env::var("QDRANT_URL_FOR_TEST") else {
            panic!("set QDRANT_URL_FOR_TEST to the base URL of a running Qdrant");
        };
        let coll = "restore_round_trip";
        let http = Client::new();

        // Fresh collection every run, so a previous failure cannot make this
        // one pass.
        let _ = http.delete(format!("{url}/collections/{coll}")).send().await;
        let created = http
            .put(format!("{url}/collections/{coll}"))
            .json(&serde_json::json!({ "vectors": { "size": 4, "distance": "Cosine" } }))
            .send().await.unwrap();
        assert!(created.status().is_success(), "creating the collection: {:?}", created.text().await);

        let upsert = |points: serde_json::Value| {
            let http = http.clone();
            let url = url.clone();
            async move {
                http.put(format!("{url}/collections/{coll}/points?wait=true"))
                    .json(&serde_json::json!({ "points": points }))
                    .send().await.unwrap()
            }
        };

        let resp = upsert(serde_json::json!([
            { "id": 1, "vector": [0.1, 0.2, 0.3, 0.4], "payload": { "filename": "archived.pdf" } },
            { "id": 2, "vector": [0.5, 0.6, 0.7, 0.8], "payload": { "filename": "also-archived.pdf" } },
        ])).await;
        assert!(resp.status().is_success(), "seeding points: {:?}", resp.text().await);

        let dir = tempfile::tempdir().unwrap();
        let db = pool_at(&dir.path().join("live.db")).await;
        sqlx::query("CREATE TABLE documents (id INTEGER PRIMARY KEY, filename TEXT)")
            .execute(&db).await.unwrap();
        sqlx::query("INSERT INTO documents (id, filename) VALUES (1, 'archived.pdf')")
            .execute(&db).await.unwrap();

        let backup_dir = dir.path().join("backups");
        let archive = create_backup(&db, "", &url, coll, backup_dir.to_str().unwrap())
            .await
            .expect("create_backup");
        let archive_name = archive.file_name().unwrap().to_str().unwrap().to_owned();

        // Now diverge from the archive on both sides.
        let resp = upsert(serde_json::json!([
            { "id": 3, "vector": [0.9, 0.9, 0.9, 0.9], "payload": { "filename": "added-after.pdf" } },
        ])).await;
        assert!(resp.status().is_success());
        sqlx::query("INSERT INTO documents (id, filename) VALUES (2, 'added-after.pdf')")
            .execute(&db).await.unwrap();

        let report = restore_backup(&db, &url, coll, backup_dir.to_str().unwrap(), &archive_name)
            .await
            .expect("restore_backup");
        assert!(report.verified, "the archive must carry a manifest and pass it");
        assert!(report.qdrant_restored, "the archive must have carried a snapshot");
        assert_eq!(report.sqlite_tables, vec!["documents".to_string()]);

        // Qdrant: back to the two archived points, without the third.
        let count: serde_json::Value = http
            .post(format!("{url}/collections/{coll}/points/count"))
            .json(&serde_json::json!({ "exact": true }))
            .send().await.unwrap()
            .json().await.unwrap();
        assert_eq!(
            count["result"]["count"].as_u64(),
            Some(2),
            "the point added after the backup must be gone: {count}"
        );

        // SQLite: same story.
        let names: Vec<String> = sqlx::query_scalar("SELECT filename FROM documents ORDER BY id")
            .fetch_all(&db).await.unwrap();
        assert_eq!(names, vec!["archived.pdf".to_string()]);

        let _ = http.delete(format!("{url}/collections/{coll}")).send().await;
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
