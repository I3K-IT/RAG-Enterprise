//! Backup creation: SQLite VACUUM INTO + Qdrant snapshot → tar.zstd archive.
//!
//! Community Python backup_service mirrors:
//! 1. SQLite: VACUUM INTO for a consistent copy (WAL-safe)
//! 2. Qdrant: POST /collections/{name}/snapshots → GET snapshot download
//! 3. Pack both into a tar.zst archive under backup_dir
//! 4. Returns path to the created archive

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use flate2::Compression;
use flate2::write::GzEncoder;
use reqwest::Client;
use serde::Deserialize;
use sqlx::SqlitePool;

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
