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
///
/// `retain_last` caps how many newest archives survive: after a successful
/// backup, anything older beyond that count is pruned. `0` disables
/// pruning and keeps the historical accumulate-forever behaviour.
pub async fn create_backup(
    db: &SqlitePool,
    _db_path: &str,
    qdrant_url: &str,
    qdrant_collection: &str,
    backup_dir: &str,
    retain_last: u64,
) -> Result<PathBuf> {
    let dir = Path::new(backup_dir);
    std::fs::create_dir_all(dir).with_context(|| format!("create backup dir {}", dir.display()))?;

    let ts = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let (work_dir, archive_path) = backup_run_paths(dir, &ts);
    std::fs::create_dir_all(&work_dir)?;
    // Owns work_dir from here on, so every failure below cleans up after
    // itself. It holds a whole copy of the database and possibly a
    // multi-hundred-megabyte Qdrant snapshot, and nothing else reclaims it:
    // list_backups only matches `*.tar.gz`, and prune_old_backups only
    // deletes what that listing returned, so a leaked work directory is
    // outside both the listing and the retention quota. On a volume that is
    // filling up because of the leak, the failures then become
    // self-sustaining - which is the opposite of what a backup should do when
    // the disk is tight.
    let _work_dir_guard = RemoveDirOnDrop(work_dir.clone());

    // 1. SQLite: VACUUM INTO for a WAL-safe consistent snapshot, then ask
    //    SQLite whether what came out is actually a sound database.
    let sqlite_dest = work_dir.join("rag_users.db");
    // VACUUM INTO takes a string literal, not a bound parameter, and SQLite has
    // no backslash escape: an apostrophe in the path terminated the literal
    // there and the statement failed. So any deployment under, say,
    // C:\Users\O'Neill\... or an iCloud/OneDrive folder with one could not be
    // backed up at all - and from the 02:00 scheduler the only symptom was a
    // single error line a day. Escaped exactly as restore_sqlite below does.
    let sqlite_dest_literal = sqlite_dest.display().to_string().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{sqlite_dest_literal}'"))
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

    // 4. Pack work_dir into a tar.gz archive. The archive file exists from
    //    the first byte written, so a failure here leaves a half-written
    //    `.tar.gz` that the listing would offer for restore and retention
    //    would count as a backup: remove it before reporting the error. The
    //    guard above still takes the work directory.
    //
    //    Taken under PRUNE_LOCK, which the retention pass below then reuses
    //    rather than taking again. The archive has to be written inside the
    //    critical section, not just pruned inside it: a competing run that
    //    lists and deletes between this write and our own pass sees an
    //    archive it does not know about, and with a small retain_last it is
    //    the one that goes - after which this run pins nothing, returns
    //    Ok(archive_path), and the admin is told a backup exists at a path
    //    that has just been deleted.
    let _serialized = PRUNE_LOCK.lock().await;
    if let Err(e) = pack_tar_gz(&work_dir, &archive_path) {
        let _ = std::fs::remove_file(&archive_path);
        return Err(e);
    }

    tracing::info!(
        archive = %archive_path.display(),
        with_vectors = manifest.qdrant.is_some(),
        "backup complete"
    );

    // 5. Retention, success-only: a failed backup must never delete older
    //    archives, and a pruning failure must never fail the backup. The
    //    archive we just wrote is named explicitly so this run can never
    //    prune its own output — see `prune_old_backups`.
    let just_written = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    prune_old_backups(backup_dir, retain_last, just_written, &_serialized).await;
    drop(_serialized);

    Ok(archive_path)
}

/// Held from before one run writes its archive until its own retention pass
/// has finished, so list-and-delete never overlaps another run's write — see
/// `create_backup` and `prune_old_backups`.
static PRUNE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Delete `backup_*` archives beyond the newest `retain_last`, oldest
/// first. `0` keeps everything. Only our own prefix is ever removed, so a
/// foreign `.tar.gz` sitting in the same directory is left alone; removal
/// failures are logged, never propagated.
///
/// Retention passes are serialized against each other, and so is the write
/// they follow. Two runs finishing together — the 02:00 cron tick and an admin
/// "Run Backup Now" — would otherwise each list the same pair of archives, each
/// pin a *different* `just_written` to the front, and each delete the other's,
/// leaving no backup at all. Listing and the deletions it implies are therefore
/// one critical section, and `create_backup` takes the same lock *before* it
/// writes, so a pass can never delete an archive another run is in the middle
/// of producing. A process-wide lock is enough: the two callers are the
/// scheduler task and the admin handler inside one process, and startup kills
/// any stale instance of the same binary before serving.
///
/// `just_written` is the archive the calling run has just produced, and it
/// always survives. That is not redundant with "keep the newest": the
/// listing sorts on the file name, whose timestamp has one-second
/// granularity, so two archives written in the same second are ordered by
/// the random suffix `backup_run_paths` appends — by chance, not by age. A
/// second run landing inside the same second as an earlier one (the
/// double-clicked "Run Backup Now" that suffix exists for) could therefore
/// sort below it and, with a small `retain_last`, delete the archive it had
/// just created while still returning its path to the caller. Pass an empty
/// name when there is no such archive to protect.
///
/// `held` is the caller's guard on `PRUNE_LOCK`, taken before its write: the
/// lock is deliberately not taken here as well, since a second acquisition
/// would deadlock on itself.
async fn prune_old_backups(
    backup_dir: &str,
    retain_last: u64,
    just_written: &str,
    held: &tokio::sync::MutexGuard<'_, ()>,
) {
    if retain_last == 0 {
        return;
    }
    let _held = held;
    // Only our own prefix counts toward the quota: a foreign `.tar.gz`
    // sitting in the same directory must neither be deleted nor consume a
    // retained slot. `list_backups` already sorts newest first, and the
    // filter preserves that order.
    let mut ours: Vec<String> = list_backups(backup_dir)
        .await
        .into_iter()
        .filter(|name| name.starts_with("backup_"))
        .collect();
    // Pin the run's own archive to the front: it is the newest by
    // construction even when a same-second sibling sorts above it.
    if let Some(i) = ours.iter().position(|name| name == just_written) {
        let own = ours.remove(i);
        ours.insert(0, own);
    }
    let mut pruned = 0u64;
    for stale in ours.iter().skip(retain_last as usize) {
        match tokio::fs::remove_file(Path::new(backup_dir).join(stale)).await {
            Ok(()) => pruned += 1,
            Err(e) => {
                tracing::warn!(archive = %stale, error = %e, "backup retention: could not remove old archive")
            }
        }
    }
    if pruned > 0 {
        tracing::info!(pruned, retain_last, "backup retention pruned old archives");
    }
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

/// Owns a backup's work directory: dropping it removes the directory, so
/// every early return cleans up without a `remove_dir_all` on each path.
///
/// The same shape as `RemoveOnDrop` in api/documents.rs, for the same
/// reason. Drop is not async, so this is the one `std::fs` call left on the
/// path; a recursive unlink is metadata work rather than a transfer, so the
/// executor stall is syscalls and not the size of the payload.
struct RemoveDirOnDrop(PathBuf);

impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            // Not the run's failure: by the time this runs the archive may
            // already be written, and the caller must not be told the backup
            // failed because a leftover directory could not be deleted. But
            // that directory holds a full copy of the database, so it is
            // worth saying out loud.
            tracing::warn!(
                path = %self.0.display(),
                error = %e,
                "backup work directory could not be removed: it holds a full copy of \
                 the database and no retention pass will reclaim it"
            );
        }
    }
}

/// Work dir + archive path for one backup run inside `dir`.
///
/// Split out so the uniqueness rule is testable without running a whole
/// backup. `ts` has one-second granularity, which is NOT unique: two runs
/// started in the same second (a double-clicked "Run Backup Now", a manual
/// run landing on the 02:00 cron tick) would otherwise share both paths,
/// interleaving their work dirs and having the second silently overwrite
/// the first's archive. The short random suffix makes every run unique
/// while keeping the timestamp prefix the listing sorts on.
fn backup_run_paths(dir: &Path, ts: &str) -> (PathBuf, PathBuf) {
    let uniq = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let stem = format!("backup_{ts}_{uniq}");
    (dir.join(&stem), dir.join(format!("{stem}.tar.gz")))
}

/// Pack the directory at `src` into a tar.gz at `dest`.
fn pack_tar_gz(src: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::create(dest)
        .with_context(|| format!("create archive {}", dest.display()))?;
    let gz = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all(".", src)
        .with_context(|| format!("tar append {}", src.display()))?;
    // finish() writes the tar end-of-archive marker and nothing else: the
    // gzip stream is finalised by GzEncoder, and letting that happen in Drop
    // threw the result away (`impl Drop for GzEncoder { try_finish() }`, whose
    // error is discarded). A volume that filled during the last few KB
    // therefore produced a truncated gzip that still decompressed to a
    // perfectly plausible tar prefix, and this returned Ok - so the run logged
    // "backup complete" and went on to prune, and the corrupt archive - the
    // newest, so pinned first - was what the listing offered to restore while
    // the good archives behind it were deleted.
    let gz = tar.into_inner().context("finishing the tar stream")?;
    // finish() writes the deflate final block and the CRC32/ISIZE trailer,
    // and reports a write that failed doing so. Then sync: the point of a
    // backup is that the bytes are on the disk, not only in the page cache,
    // and nothing else here would notice a write that never landed.
    let file = gz.finish().context("finishing the gzip stream")?;
    file.sync_all()
        .with_context(|| format!("flushing the archive {}", dest.display()))?;
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
    // The status is checked before the body is parsed, exactly as
    // upload_qdrant_snapshot below does. Without it a 401 (bad api-key), a 404
    // (no such collection) or a 500 has its body handed to serde, which fails
    // with "error decoding response body" - the status is gone, and the
    // operator is left reading a serde message instead of the one line that
    // says why. That also puts a Qdrant-side failure into the same reqwest
    // error kind as a broken body, which is neither connect, timeout nor
    // request, so is_qdrant_unreachable does not classify it as "did not
    // answer" - which is right, but only if the reason is legible.
    let resp: SnapshotResult = http
        .post(&url)
        .send()
        .await
        .context("qdrant snapshot POST")?
        .error_for_status()
        .with_context(|| format!("qdrant refused to create a snapshot of {collection}"))?
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
        .error_for_status()
        .with_context(|| format!("qdrant refused to send snapshot {snap_name} of {collection}"))?
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

    // Now that our copy is proven, the copy Qdrant kept is not needed again.
    // Nothing in this project deletes it: `retain_last` prunes local tarballs
    // only, so every daily backup left a whole collection-sized snapshot in
    // the Qdrant host's snapshots/ directory, growing without bound until that
    // volume filled - and then the snapshot POST began failing, which aborts
    // the backup the caller asked for. The backup subsystem filling the disk it
    // is trying to protect.
    //
    // Best-effort, and deliberately after the verification above: an
    // unverified download must leave the server-side copy in place so the run
    // can be retried. A failure here is worth saying out loud - the space is
    // still held - but it is not a backup failure, and must not fail one.
    match http
        .delete(&dl_url)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
    {
        Ok(_) => tracing::info!(snapshot = %snap_name, "server-side Qdrant snapshot removed"),
        Err(e) => tracing::warn!(
            snapshot = %snap_name,
            error = %e,
            "the server-side Qdrant snapshot could not be removed: it is still taking up \
             space in the Qdrant host, and no retention pass will reclaim it"
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
        // The name comes out of the archive, so it is held to being a plain
        // file name before it is joined: an absolute path would make `join`
        // throw the base away and point the digest at a file anywhere on the
        // host, and its size and sha256 would then be reported in the mismatch
        // message below - an oracle for the content of any file this process
        // can read, reachable by anyone who can hand the admin a .tar.gz. The
        // restore applies the same rule to the same two names.
        if !is_plain_file_name(&member.file) {
            anyhow::bail!(
                "the archive's manifest names {:?}, which is not a plain file name",
                member.file
            );
        }
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
    let manifest = match verify_unpacked(tmp.path())? {
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
            Some(manifest)
        }
        None => {
            tracing::warn!(
                archive = archive_name,
                "archive written before 0.1.27: it carries no manifest, so its contents cannot be \
                 verified before being restored"
            );
            None
        }
    };

    // The member names come from the manifest, the same ones verify_unpacked
    // just hashed. Reconstructing them from the current configuration instead -
    // which is what this used to do - is how a restore ended up verifying one
    // set of files and applying another: rename QDRANT__COLLECTION, or move
    // the archive to another install, and the snapshot is verified under the
    // name it was taken with but then looked for under the one configured now.
    // It was not found, the run reported success, and only the database came
    // back.
    let sqlite_name = manifest
        .as_ref()
        .map(|m| m.sqlite.file.clone())
        .unwrap_or_else(|| "rag_users.db".to_owned());
    let snapshot_name = manifest
        .as_ref()
        .and_then(|m| m.qdrant.as_ref().map(|q| q.file.clone()))
        .unwrap_or_else(|| format!("{qdrant_collection}.snapshot"));
    // The names come out of the archive, so they are held to the rule an
    // archive name from an HTTP body is: a plain file name. A manifest naming
    // `../x` or an absolute path would otherwise point the restore at a file
    // anywhere on the host, now that the restore applies what it names.
    for name in [&sqlite_name, &snapshot_name] {
        if !is_plain_file_name(name) {
            anyhow::bail!("the archive's manifest names {name:?}, which is not a plain file name");
        }
    }

    // And an archive of a different collection is refused outright rather than
    // applied to this one: uploading another collection's snapshot under this
    // name would put vectors in the wrong place, which is worse than not
    // restoring them. Nothing above this point has been written yet.
    if let Some(taken_from) = manifest
        .as_ref()
        .and_then(|m| m.qdrant_collection.as_deref())
    {
        if taken_from != qdrant_collection {
            anyhow::bail!(
                "this archive holds the Qdrant collection {taken_from:?}, but this \
                 installation is configured for {qdrant_collection:?}: restoring it would put \
                 the vectors in the wrong collection. Nothing has been restored."
            );
        }
    }

    // The database is looked for before anything is written. A manifest always
    // promises it, and verify_unpacked has made sure it is there, but an archive
    // from before 0.1.27 carries no manifest: finding its database missing only
    // after the snapshot upload would leave the vectors replaced and the
    // database not, while reporting that nothing was restored. (It used to be
    // reported as a success, with an empty report and nothing in the log.)
    let sqlite = tmp.path().join(&sqlite_name);
    if !sqlite.is_file() {
        anyhow::bail!("the archive contains no {sqlite_name}: nothing was restored");
    }

    let snapshot = tmp.path().join(&snapshot_name);
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

    let (tables, rows) = restore_sqlite(db, &sqlite).await?;
    tracing::info!(tables = tables.len(), rows, "SQLite restored");
    report.sqlite_tables = tables;
    report.sqlite_rows = rows;

    Ok(report)
}

/// Resolve `name` to an archive inside `backup_dir`.
///
/// The name arrives in an HTTP body, so it is treated as hostile: anything
/// carrying a separator or a parent component could point the unpacker at an
/// arbitrary file on the host.
fn resolve_archive(backup_dir: &str, name: &str) -> Result<PathBuf> {
    if !is_plain_file_name(name) {
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

        // The path check above says nothing about WHAT the entry is: a
        // symlink or hard link with an innocent path still materialises an
        // arbitrary target on unpack (and a restored symlink can redirect a
        // later write outside `dest`). Our own archives only ever contain
        // regular files (and, harmlessly, directories), so refuse the rest
        // outright rather than inspecting link targets.
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            anyhow::bail!(
                "archive entry is not a regular file or directory: {}",
                path.display()
            );
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

    // Reaching the tar end-of-archive marker does not finish the gzip stream:
    // flate2 has still not read the CRC32/ISIZE trailer. So a stream cut short
    // after the tar was complete - which is exactly what a failed
    // finalisation on the writing side produces - unpacked "successfully", and
    // the members that had made it were silently all the archive contained.
    // Draining the decoder to EOF is what proves the archive is whole: a
    // missing or mismatched trailer surfaces as a read error here.
    let mut gz = tar.into_inner();
    std::io::copy(&mut gz, &mut std::io::sink())
        .context("the archive's gzip stream is truncated or its checksum does not match")?;
    Ok(())
}

/// An entry path is safe when it is relative and walks only downwards.
fn is_safe_entry_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// A name is a plain file name when joining it onto a directory can only ever
/// produce a child of that directory: one `Normal` component, so no separator,
/// no `..`, and nothing `join` would treat as absolute.
///
/// The same rule as `resolve_archive` applies to an archive name arriving in an
/// HTTP body, and the same rule the restore applies to the member names a
/// manifest carries. All three are strings from outside the program, and each
/// is joined onto a directory this code then reads.
///
/// A backslash is refused outright: it is a separator on Windows and an
/// ordinary character everywhere else, so allowing it would let the same name
/// be a child on one platform and a path on another. None of the names this
/// code produces contains one.
fn is_plain_file_name(name: &str) -> bool {
    !name.contains('\\')
        && Path::new(name).components().collect::<Vec<_>>().as_slice()
            == [Component::Normal(name.as_ref())]
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

    // The three statements below put the connection back the way it was found.
    // They were best-effort only while they succeed: if the ROLLBACK fails -
    // the same SQLITE_BUSY or SQLITE_FULL that made the copy fail - the write
    // transaction is still open, the DETACH cannot succeed while a transaction
    // is open, and `PRAGMA foreign_keys = ON` is a silent no-op inside one.
    // Handing that back would put a connection into the pool holding the
    // RESERVED lock with foreign-key enforcement off, which is exactly what
    // these lines exist to prevent, and every later user of that connection
    // would inherit both with nothing to show why.
    //
    // So each step is checked, and a connection that could not be put back is
    // closed rather than returned.
    let mut dirty = false;
    if result.is_err() {
        // Undo the half-written copy before anything else: without this the
        // next operation on that connection - including a retried restore,
        // which cannot BEGIN inside an open transaction - fails with a
        // confusing error far from the actual cause.
        if let Err(e) = sqlx::query("ROLLBACK").execute(&mut *conn).await {
            tracing::error!(
                error = %e,
                "the restore's ROLLBACK failed: the connection is being closed rather than \
                 returned to the pool, since it still holds an open write transaction"
            );
            dirty = true;
        }
    }

    for statement in ["DETACH DATABASE backup", "PRAGMA foreign_keys = ON"] {
        if let Err(e) = sqlx::query(statement).execute(&mut *conn).await {
            tracing::error!(
                statement,
                error = %e,
                "a restore connection could not be put back the way it was found"
            );
            dirty = true;
        }
    }
    if dirty {
        // The restore's own result is what the caller gets either way: a close
        // that fails as well is worth the log line, not a different error.
        if let Err(e) = conn.close().await {
            tracing::error!(
                error = %e,
                "closing a restore connection that could not be reset failed as well"
            );
        }
    }

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
///
/// Async because it is called straight from an axum handler: a synchronous
/// read_dir there blocks the executor thread for as long as the directory
/// takes to walk, which on a network mount or a directory holding hundreds
/// of archives is long enough to stall every other request sharing that
/// worker. An unreadable directory still answers with an empty list, as
/// before — a missing backup dir is the normal state before the first
/// backup, not an error worth surfacing.
pub async fn list_backups(backup_dir: &str) -> Vec<String> {
    let Ok(mut entries) = tokio::fs::read_dir(Path::new(backup_dir)).await else {
        return vec![];
    };
    let mut names = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tar.gz") {
            names.push(name);
        }
    }
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

    // ── backup runs ─────────────────────────────────────────────────────────

    /// Same timestamp twice must still yield different paths: one-second
    /// granularity is not unique across runs.
    #[test]
    fn same_second_runs_get_different_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (work_a, archive_a) = backup_run_paths(dir.path(), "20260101_000000");
        let (work_b, archive_b) = backup_run_paths(dir.path(), "20260101_000000");
        assert_ne!(work_a, work_b);
        assert_ne!(archive_a, archive_b);
        // The timestamp prefix stays, so the newest-first listing order is
        // unaffected; only the .tar.gz archive keeps its extension.
        for p in [&work_a, &work_b, &archive_a, &archive_b] {
            assert!(
                p.file_name().unwrap().to_str().unwrap().starts_with("backup_20260101_000000_"),
                "unexpected name: {}",
                p.display()
            );
        }
        assert_eq!(archive_a.extension().unwrap(), "gz");
    }

    /// The reported scenario: two backups back to back keep two archives.
    /// Unreachable Qdrant still writes a (database-only) archive, so no
    /// server is needed.
    #[tokio::test]
    async fn two_backups_in_a_row_keep_two_archives() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        let backup_dir = d.path().join("backups");
        let unreachable = "http://127.0.0.1:9";
        let first = create_backup(
            &live,
            "",
            unreachable,
            "rag_documents",
            backup_dir.to_str().unwrap(),
            0,
        )
        .await
        .unwrap();
        let second = create_backup(
            &live,
            "",
            unreachable,
            "rag_documents",
            backup_dir.to_str().unwrap(),
            0,
        )
        .await
        .unwrap();
        assert_ne!(first, second);
        assert!(first.is_file() && second.is_file(), "second run must not overwrite the first");
        assert_eq!(list_backups(backup_dir.to_str().unwrap()).await.len(), 2);
    }

    /// A backup directory whose path contains an apostrophe. `VACUUM INTO`
    /// takes a string literal and SQLite has no backslash escape, so the
    /// unescaped path ended the literal at the quote and the whole backup
    /// failed - for every run, from a path a user chooses.
    #[tokio::test]
    async fn a_backup_directory_containing_an_apostrophe_still_works() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(&live)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'O''Neill')")
            .execute(&live)
            .await
            .unwrap();

        let backup_dir = d.path().join("O'Neill's backups");
        let archive = create_backup(
            &live,
            "",
            // Port 9: nothing listens, so this takes the database-only path
            // without needing a Qdrant.
            "http://127.0.0.1:9",
            "rag_documents",
            backup_dir.to_str().unwrap(),
            0,
        )
        .await
        .expect("a path with an apostrophe must not break the backup");
        assert!(archive.is_file(), "{}", archive.display());

        // And the copy really is a sound database holding the row.
        let copy = d.path().join("checked.db");
        std::fs::create_dir(&copy).unwrap();
        unpack_tar_gz(&archive, &copy).unwrap();
        let restored = pool_at(&copy.join("rag_users.db")).await;
        let name: String = sqlx::query_scalar("SELECT name FROM users WHERE id = 1")
            .fetch_one(&restored)
            .await
            .unwrap();
        assert_eq!(name, "O'Neill");
    }

    /// The archive is a gzip stream, and its trailer is only written when the
    /// encoder is finalised — which used to happen in `Drop`, where the error
    /// is discarded. So a failure in the last few KB produced a truncated
    /// stream that still decompressed to a plausible tar prefix, and
    /// `pack_tar_gz` returned `Ok`.
    ///
    /// Nothing read a packed archive back: `two_backups_in_a_row…` only checks
    /// `is_file()`. This one unpacks what `create_backup` produced and reads
    /// the manifest and the database out of it, which fails on a stream whose
    /// trailer is missing.
    #[tokio::test]
    async fn the_archive_packed_is_one_that_can_be_read_back() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(&live)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'Ada')")
            .execute(&live)
            .await
            .unwrap();

        let archive = create_backup(
            &live,
            "",
            "http://127.0.0.1:9",
            "rag_documents",
            d.path().join("backups").to_str().unwrap(),
            0,
        )
        .await
        .unwrap();

        let out = d.path().join("unpacked");
        std::fs::create_dir(&out).unwrap();
        unpack_tar_gz(&archive, &out).expect("the archive we just wrote must unpack");

        let manifest: BackupManifest =
            serde_json::from_slice(&std::fs::read(out.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.format, MANIFEST_FORMAT);
        assert_eq!(manifest.sqlite.size, std::fs::metadata(out.join("rag_users.db")).unwrap().len());

        // Losing the tail is what a failed gzip finalisation on the writing
        // side looks like on disk: the tar inside is complete, the container
        // is not. Reading the members is not enough to notice - the trailer is
        // only consulted once the stream is drained.
        let whole = std::fs::read(&archive).unwrap();
        let cut = whole.len() - 4;
        std::fs::write(&archive, &whole[..cut]).unwrap();
        let out2 = d.path().join("unpacked2");
        std::fs::create_dir(&out2).unwrap();
        let err = unpack_tar_gz(&archive, &out2)
            .expect_err("an archive whose gzip trailer is missing must not read as complete");
        assert!(format!("{err:#}").contains("gzip stream is truncated"), "{err:#}");
    }

    /// A Qdrant that answers every snapshot request with `status` and `body`.
    /// Binds a random port and serves until the test ends.
    async fn qdrant_answering(status: axum::http::StatusCode, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                axum::Router::new().fallback(move || async move { (status, body) }),
            )
            .await;
        });
        format!("http://{addr}")
    }

    /// Entries in the backup directory that are not archives, i.e. leftovers.
    fn work_dirs_left_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with(".tar.gz"))
            .collect()
    }

    /// A Qdrant that answers 500: the snapshot is unverifiable, so the run
    /// aborts *after* the work directory holds a complete copy of the
    /// database. That directory has to go with the failure, because nothing
    /// else will: `list_backups` only matches `*.tar.gz`, and
    /// `prune_old_backups` only deletes what the listing returned, so a
    /// leftover `backup_<ts>_<uniq>/` sits outside the retention quota
    /// forever.
    #[tokio::test]
    async fn a_failed_backup_does_not_leave_its_work_directory_behind() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(&live)
            .await
            .unwrap();
        let qdrant = qdrant_answering(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "no snapshot for you",
        )
        .await;
        let backup_dir = d.path().join("backups");

        let result = create_backup(
            &live,
            "",
            &qdrant,
            "rag_documents",
            backup_dir.to_str().unwrap(),
            0,
        )
        .await;
        assert!(
            result.is_err(),
            "a Qdrant that answers 500 must abort the backup"
        );
        // Nothing published, so no archive either.
        assert!(list_backups(backup_dir.to_str().unwrap()).await.is_empty());
        assert_eq!(
            work_dirs_left_in(&backup_dir),
            Vec::<String>::new(),
            "a failed backup left its work directory - a full copy of the database - \
             where neither the listing nor retention will ever see it"
        );
    }

    /// The same directory is also gone after a success, so the guard did not
    /// simply move the cleanup somewhere that stops running.
    #[tokio::test]
    async fn a_successful_backup_leaves_only_its_archive() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        let backup_dir = d.path().join("backups");
        let archive = create_backup(
            &live,
            "",
            "http://127.0.0.1:9",
            "rag_documents",
            backup_dir.to_str().unwrap(),
            0,
        )
        .await
        .unwrap();
        assert!(archive.is_file());
        assert_eq!(work_dirs_left_in(&backup_dir), Vec::<String>::new());
    }

    /// A snapshot that cannot be taken has to say which HTTP status said so.
    /// The body Qdrant sends for a missing collection is a JSON error object,
    /// which is not a `SnapshotResult`, so before the status was checked the
    /// failure surfaced as "error decoding response body" and the 404 was
    /// gone — the operator got a serde message and no idea whether the
    /// collection was missing, the api-key wrong, or Qdrant broken.
    #[tokio::test]
    async fn a_refused_snapshot_names_the_status_that_refused_it() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        let qdrant = qdrant_answering(
            axum::http::StatusCode::NOT_FOUND,
            r#"{"status":{"error":"Not found: collection `rag_documents` doesn't exist!"}}"#,
        )
        .await;

        let err = create_backup(
            &live,
            "",
            &qdrant,
            "rag_documents",
            d.path().join("backups").to_str().unwrap(),
            0,
        )
        .await
        .expect_err("a Qdrant that refuses the snapshot must abort the backup");

        let reported = format!("{err:#}");
        assert!(
            reported.contains("404"),
            "the status must survive into the report, got: {reported}"
        );
        assert!(
            !reported.contains("error decoding response body"),
            "the serde error must not be what the operator reads, got: {reported}"
        );
        // Still a hard failure rather than the survivable "Qdrant did not
        // answer" case: a refusal is an answer.
        assert!(
            !is_qdrant_unreachable(&err),
            "a status error is not an unreachable Qdrant"
        );
    }

    /// A Qdrant that hands out a real, verifiable snapshot and records whether
    /// it was asked to delete it afterwards.
    async fn qdrant_serving_a_snapshot(
        deleted: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> (String, Vec<u8>) {
        let bytes = b"not a real snapshot, but a real one to checksum".to_vec();
        let sha = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(&bytes))
        };
        let size = bytes.len() as u64;
        let meta = serde_json::json!({
            "result": { "name": "backup.snapshot", "size": size, "checksum": sha }
        });
        let meta_body = meta.to_string();
        let body = bytes.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let serve = move |method: axum::http::Method| {
                let deleted = deleted.clone();
                let body = body.clone();
                let meta_body = meta_body.clone();
                async move {
                    let payload = match method {
                        axum::http::Method::POST => meta_body,
                        axum::http::Method::DELETE => {
                            deleted.store(true, std::sync::atomic::Ordering::SeqCst);
                            String::new()
                        }
                        _ => String::from_utf8_lossy(&body).into_owned(),
                    };
                    (axum::http::StatusCode::OK, payload)
                }
            };
            let _ = axum::serve(listener, axum::Router::new().fallback(serve)).await;
        });
        (format!("http://{addr}"), bytes)
    }

    /// Qdrant keeps a copy of every snapshot it creates, and nothing in this
    /// project deleted it: `retain_last` prunes local tarballs only. So each
    /// daily backup added a collection-sized snapshot to the Qdrant host,
    /// growing without bound until that volume filled - and then the snapshot
    /// POST started failing, which aborts the very backup meant to protect
    /// against exactly this. The copy on the Qdrant host is dropped once ours
    /// is verified.
    #[tokio::test]
    async fn the_snapshot_is_deleted_from_qdrant_once_ours_is_verified() {
        let d = tempfile::tempdir().unwrap();
        let deleted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (qdrant, bytes) = qdrant_serving_a_snapshot(deleted.clone()).await;
        let dest = d.path().join("work");
        std::fs::create_dir(&dest).unwrap();

        let digest = create_qdrant_snapshot(&qdrant, "rag_documents", &dest)
            .await
            .expect("the snapshot we served verifies against its own checksum");
        assert_eq!(digest.size, bytes.len() as u64);
        assert!(
            deleted.load(std::sync::atomic::Ordering::SeqCst),
            "the snapshot Qdrant kept was never deleted, so every backup leaves another \
             one on its host"
        );
    }

    // ── retention ───────────────────────────────────────────────────────────

    /// A run must never hand back a path to a file that is already gone. The
    /// write was outside the retention lock, so a competing run could list
    /// and delete in the window between this run's pack and its own pass,
    /// take the fresh archive with it, and leave this run returning `Ok` for a
    /// path that no longer exists — the admin endpoint answering
    /// `{"ok": true}` for a backup that was just deleted.
    ///
    /// The check is made inside each task, immediately after the call
    /// returns: that is the moment the claim is made, and with the lock held a
    /// second run cannot have reached its own pass yet. Asserting it after
    /// joining both would be wrong — with `retain_last = 1` a *later* run
    /// pruning the earlier archive is the policy working, not a fault.
    ///
    /// A handful of rounds, not a hundred: each one is two whole backups, and
    /// the collision needs only a scheduler tick and a double-clicked button.
    #[tokio::test]
    async fn a_run_never_returns_a_path_already_deleted_by_a_competing_run() {
        for round in 0..6 {
            let d = tempfile::tempdir().unwrap();
            let live = pool_at(&d.path().join("live.db")).await;
            sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
                .execute(&live)
                .await
                .unwrap();
            let backup_dir = path_of(&d);
            let (p1, p2) = (backup_dir.clone(), backup_dir.clone());
            let (b1, b2) = (live.clone(), live.clone());

            // retain_last = 1, so whichever run prunes second removes the
            // other's archive - which is exactly the collision.
            let run = |db: SqlitePool, dir: String| async move {
                let archive =
                    create_backup(&db, "", "http://127.0.0.1:9", "rag_documents", &dir, 1)
                        .await
                        .expect("the run itself must succeed");
                (archive.display().to_string(), archive.is_file())
            };
            let a = tokio::spawn(run(b1, p1));
            let b = tokio::spawn(run(b2, p2));
            let (one, two) = tokio::join!(a, b);

            for (path, there) in [one.unwrap(), two.unwrap()] {
                assert!(
                    there,
                    "round {round}: a run returned {path}, which was already gone: a \
                     competing run pruned it between the write and the retention pass"
                );
            }
            // And the policy itself still holds: two runs, one kept.
            assert_eq!(
                list_backups(&backup_dir).await.len(),
                1,
                "round {round}: retain_last = 1 over two runs must leave one archive"
            );
        }
    }

    fn fake_archive(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"not really an archive").unwrap();
    }

    /// One retention pass, taken the way `create_backup` takes it: the lock
    /// first, then the pass. A test that called the pass without the lock
    /// would not be exercising the ordering the real run has.
    async fn prune(dir: &str, retain_last: u64, just_written: &str) {
        let held = PRUNE_LOCK.lock().await;
        prune_old_backups(dir, retain_last, just_written, &held).await;
    }

    fn path_of(dir: &tempfile::TempDir) -> String {
        dir.path().to_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn retention_zero_disables_pruning() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=3 {
            fake_archive(dir.path(), &format!("backup_2026010{n}_000000_aaa{n}1111.tar.gz"));
        }
        prune(dir.path().to_str().unwrap(), 0, "").await;
        assert_eq!(list_backups(dir.path().to_str().unwrap()).await.len(), 3);
    }

    #[tokio::test]
    async fn retention_keeps_the_newest_and_spares_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=4 {
            fake_archive(dir.path(), &format!("backup_2026010{n}_000000_aaa{n}1111.tar.gz"));
        }
        fake_archive(dir.path(), "someone-elses-archive.tar.gz");
        prune(
            dir.path().to_str().unwrap(),
            2,
            "backup_20260104_000000_aaa41111.tar.gz",
        )
        .await;
        let mut names = list_backups(dir.path().to_str().unwrap()).await;
        names.sort();
        assert_eq!(
            names,
            vec![
                "backup_20260103_000000_aaa31111.tar.gz".to_string(),
                "backup_20260104_000000_aaa41111.tar.gz".to_string(),
                "someone-elses-archive.tar.gz".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn three_backups_with_retain_two_leave_two_archives() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        let backup_dir = d.path().join("backups");
        let unreachable = "http://127.0.0.1:9";
        for _ in 0..3 {
            create_backup(&live, "", unreachable, "rag_documents", backup_dir.to_str().unwrap(), 2)
                .await
                .unwrap();
        }
        assert_eq!(list_backups(backup_dir.to_str().unwrap()).await.len(), 2);
    }

    /// Two archives inside the same second are ordered by the random suffix,
    /// not by age, so the one a run has just written can sort below an older
    /// sibling. Without the `just_written` guard, `retain_last = 1` deletes it
    /// and `create_backup` returns the path of a file that no longer exists.
    #[tokio::test]
    async fn a_run_never_prunes_the_archive_it_just_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let older = "backup_20260101_000000_ffffffff.tar.gz";
        let just_written = "backup_20260101_000000_00000000.tar.gz";
        fake_archive(dir.path(), older);
        fake_archive(dir.path(), just_written);

        prune(dir.path().to_str().unwrap(), 1, just_written).await;

        assert_eq!(
            list_backups(dir.path().to_str().unwrap()).await,
            vec![just_written.to_string()],
            "the surviving archive must be the one this run wrote, not the \
             same-second sibling that happens to sort above it"
        );
    }

    /// Two runs finishing together each pin their own archive, so without a
    /// lock around the pass each deletes the other's and the directory is
    /// left empty — measured at 196 rounds out of 200 before `PRUNE_LOCK`.
    /// Retention may never take the count below `retain_last`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_retention_passes_never_empty_the_directory() {
        for round in 0..64 {
            let dir = tempfile::tempdir().unwrap();
            let one = "backup_20260101_000000_aaaaaaaa.tar.gz";
            let two = "backup_20260101_000000_bbbbbbbb.tar.gz";
            fake_archive(dir.path(), one);
            fake_archive(dir.path(), two);

            let (p1, p2) = (path_of(&dir), path_of(&dir));
            let a = tokio::spawn(async move { prune(&p1, 1, one).await });
            let b = tokio::spawn(async move { prune(&p2, 1, two).await });
            let _ = tokio::join!(a, b);

            assert_eq!(
                list_backups(&path_of(&dir)).await.len(),
                1,
                "round {round}: two competing retention passes must leave \
                 exactly the one archive `retain_last = 1` asks for"
            );
        }
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

    // ── tar entry types ─────────────────────────────────────────────────────

    type Tarball = (tempfile::TempDir, PathBuf);

    /// Builds a gzipped tarball from raw headers (so the entry types are
    /// exactly what the test sets, not what a convenience helper derives).
    /// Returns the tempdir holding it alongside, keeping it alive.
    fn tarball_with(
        build: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>),
    ) -> Tarball {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("test.tar.gz");
        let file = std::fs::File::create(&archive).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        build(&mut builder);
        builder.into_inner().unwrap().finish().unwrap();
        (dir, archive)
    }

    fn file_header(path: &str, data_len: u64) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(data_len);
        header.set_cksum();
        header
    }

    #[test]
    fn unpack_accepts_regular_files() {
        let data = b"database";
        let (_dir, archive) = tarball_with(|b| {
            let header = file_header("rag_users.db", data.len() as u64);
            b.append(&header, &data[..]).unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        unpack_tar_gz(&archive, dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("rag_users.db")).unwrap(),
            b"database"
        );
    }

    /// A symlink with an innocent path still materialises an arbitrary
    /// target on unpack — and a restored one can redirect a later write
    /// outside the destination — so the type is refused even though the
    /// path itself passes `is_safe_entry_path`.
    #[test]
    fn unpack_rejects_symlink_entries() {
        let (_dir, archive) = tarball_with(|b| {
            let mut header = tar::Header::new_gnu();
            header.set_path("link").unwrap();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("/etc/passwd").unwrap();
            header.set_size(0);
            header.set_cksum();
            b.append(&header, std::io::empty()).unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        let err = unpack_tar_gz(&archive, dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("not a regular file"),
            "unexpected error: {err:#}"
        );
        assert!(!dest.path().join("link").exists());
    }

    #[test]
    fn unpack_rejects_hardlink_entries() {
        let (_dir, archive) = tarball_with(|b| {
            let mut header = tar::Header::new_gnu();
            header.set_path("alias").unwrap();
            header.set_entry_type(tar::EntryType::Link);
            header.set_link_name("rag_users.db").unwrap();
            header.set_size(0);
            header.set_cksum();
            b.append(&header, std::io::empty()).unwrap();
        });
        let dest = tempfile::tempdir().unwrap();
        let err = unpack_tar_gz(&archive, dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("not a regular file"),
            "unexpected error: {err:#}"
        );
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

    /// An archive whose manifest says exactly this: a sound copy of `live` at
    /// `sqlite_name`, an optional snapshot at `snapshot_name`, and the
    /// collection it was taken from. Built by hand rather than through
    /// `create_backup` so a restore can be pointed at a configuration the
    /// archive was not taken under.
    async fn archive_with_manifest(
        live: &SqlitePool,
        backup_dir: &Path,
        sqlite_name: &str,
        snapshot_name: Option<&str>,
        collection: Option<&str>,
    ) -> String {
        let work = backup_dir.join("work");
        std::fs::create_dir(&work).unwrap();
        // VACUUM INTO a fixed name and then rename: the manifest may claim a
        // different one, and the point is that the manifest decides.
        let real = work.join("real.db");
        sqlx::query(&format!("VACUUM INTO '{}'", real.display()))
            .execute(live)
            .await
            .unwrap();
        let digest = digest_of(&real).unwrap();
        std::fs::rename(&real, work.join(sqlite_name)).unwrap();
        let sqlite = MemberDigest {
            file: sqlite_name.to_owned(),
            ..digest
        };

        let qdrant = snapshot_name.map(|name| {
            std::fs::write(work.join(name), b"not a real snapshot").unwrap();
            let digest = digest_of(&work.join(name)).unwrap();
            MemberDigest {
                file: name.to_owned(),
                ..digest
            }
        });

        let manifest = BackupManifest {
            format: MANIFEST_FORMAT,
            created: Utc::now().to_rfc3339(),
            engine_version: "test".to_owned(),
            sqlite,
            qdrant,
            qdrant_collection: collection.map(str::to_owned),
        };
        std::fs::write(
            work.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let archive = backup_dir.join("archive.tar.gz");
        pack_tar_gz(&work, &archive).unwrap();
        archive.file_name().unwrap().to_str().unwrap().to_owned()
    }

    /// The manifest names the collection the snapshot was taken from, and a
    /// restore into a differently-configured one used to verify the archive
    /// and then quietly restore half of it: the snapshot was hashed under the
    /// name it was written with, and then looked for under the name this
    /// installation is configured for. Not found, warning logged, the run
    /// reported success, and only the database came back. An archive of
    /// another collection is refused instead, before anything is written.
    #[tokio::test]
    async fn an_archive_of_another_collection_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE documents (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        sqlx::query("INSERT INTO documents (id) VALUES (1)")
            .execute(&live)
            .await
            .unwrap();
        let backup_dir = d.path().join("backups");
        std::fs::create_dir(&backup_dir).unwrap();
        let archive = archive_with_manifest(
            &live,
            &backup_dir,
            "rag_users.db",
            Some("rag_documents.snapshot"),
            Some("rag_documents"),
        )
        .await;

        // This installation points at a different collection now.
        let err = restore_backup(
            &live,
            "http://127.0.0.1:9",
            "rag_docs_v2",
            backup_dir.to_str().unwrap(),
            &archive,
        )
        .await
        .expect_err("an archive of another collection must not be applied to this one");
        let reported = format!("{err:#}");
        assert!(reported.contains("rag_documents"), "{reported}");
        assert!(reported.contains("rag_docs_v2"), "{reported}");

        // And nothing was restored: the live row is still the only one.
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents")
            .fetch_one(&live)
            .await
            .unwrap();
        assert_eq!(rows, 1, "a refused restore must not touch the database");
    }

    /// The same archive under its own collection name is applied, so the check
    /// above is not simply refusing every archive with a snapshot in it.
    #[tokio::test]
    async fn an_archive_of_this_collection_is_applied() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE documents (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        let backup_dir = d.path().join("backups");
        std::fs::create_dir(&backup_dir).unwrap();
        let archive = archive_with_manifest(
            &live,
            &backup_dir,
            "rag_users.db",
            Some("rag_documents.snapshot"),
            Some("rag_documents"),
        )
        .await;

        // No Qdrant: the upload fails, which is enough to show the check let
        // the restore through to the point of trying.
        let err = restore_backup(
            &live,
            "http://127.0.0.1:9",
            "rag_documents",
            backup_dir.to_str().unwrap(),
            &archive,
        )
        .await
        .expect_err("nothing listens on port 9, so the upload cannot succeed");
        assert!(
            !format!("{err:#}").contains("wrong collection"),
            "a matching collection must not be refused: {err:#}"
        );
    }

    /// The manifest is also what says where the database is. Reconstructing the
    /// name from the current configuration meant a manifest-less or renamed
    /// member was verified under one name and then applied as though it were
    /// not there at all - and, because the branch had no `else`, that was
    /// reported as a successful restore of nothing.
    #[tokio::test]
    async fn the_manifest_decides_where_the_database_is_looked_for() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE documents (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        let backup_dir = d.path().join("backups");
        std::fs::create_dir(&backup_dir).unwrap();
        // A member name nothing but the manifest knows.
        let archive = archive_with_manifest(&live, &backup_dir, "accounts.db", None, None).await;

        let report = restore_backup(
            &live,
            "http://127.0.0.1:9",
            "rag_documents",
            backup_dir.to_str().unwrap(),
            &archive,
        )
        .await
        .expect("a manifest that names the member is the authority on where it is");
        assert!(report.verified);
        assert_eq!(
            report.sqlite_tables,
            vec!["documents".to_string()],
            "the database the manifest names must be the one restored"
        );
    }

    /// An archive with no database in it is not a restore: it used to be
    /// reported as a success, with an empty report and nothing in the log. And
    /// it is refused before the snapshot upload - an archive from before 0.1.27
    /// has no manifest to promise the database, and discovering it missing only
    /// after the vectors were replaced would leave half a restore behind.
    #[tokio::test]
    async fn an_archive_with_no_database_is_an_error_not_a_successful_restore() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        let backup_dir = d.path().join("backups");
        let work = backup_dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        // No manifest, as before 0.1.27, and a snapshot but no database.
        std::fs::write(work.join("rag_documents.snapshot"), b"not a real snapshot").unwrap();
        pack_tar_gz(&work, &backup_dir.join("legacy.tar.gz")).unwrap();

        let err = restore_backup(
            &live,
            "http://127.0.0.1:9",
            "rag_documents",
            backup_dir.to_str().unwrap(),
            "legacy.tar.gz",
        )
        .await
        .expect_err("an archive without a database is not restorable");
        // Refused for the database, not failed at the upload: nothing listens
        // on port 9, so reaching the upload would have failed there instead.
        assert!(format!("{err:#}").contains("rag_users.db"), "{err:#}");
    }

    /// The manifest is read out of the archive, so the names in it are held to
    /// the rule an archive name from an HTTP body is: a plain file name. One
    /// naming an absolute path - here another database on the host, with its
    /// correct digest, so verification alone lets it through - is refused
    /// before anything is applied.
    #[tokio::test]
    async fn a_manifest_naming_a_file_outside_the_archive_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        sqlx::query("CREATE TABLE documents (id INTEGER PRIMARY KEY)")
            .execute(&live)
            .await
            .unwrap();
        sqlx::query("INSERT INTO documents (id) VALUES (1)")
            .execute(&live)
            .await
            .unwrap();
        // Another database on the same host.
        let elsewhere = d.path().join("elsewhere.db");
        let other = pool_at(&elsewhere).await;
        sqlx::query("CREATE TABLE documents (id INTEGER PRIMARY KEY)")
            .execute(&other)
            .await
            .unwrap();
        sqlx::query("INSERT INTO documents (id) VALUES (42)")
            .execute(&other)
            .await
            .unwrap();
        other.close().await;

        let backup_dir = d.path().join("backups");
        let work = backup_dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let manifest = BackupManifest {
            format: MANIFEST_FORMAT,
            created: Utc::now().to_rfc3339(),
            engine_version: "test".to_owned(),
            sqlite: MemberDigest {
                file: elsewhere.to_str().unwrap().to_owned(),
                ..digest_of(&elsewhere).unwrap()
            },
            qdrant: None,
            qdrant_collection: None,
        };
        std::fs::write(work.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
        pack_tar_gz(&work, &backup_dir.join("outside.tar.gz")).unwrap();

        let err = restore_backup(
            &live,
            "http://127.0.0.1:9",
            "rag_documents",
            backup_dir.to_str().unwrap(),
            "outside.tar.gz",
        )
        .await
        .expect_err("a manifest naming a file outside the archive must be refused");
        assert!(format!("{err:#}").contains("not a plain file name"), "{err:#}");
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM documents")
            .fetch_all(&live)
            .await
            .unwrap();
        assert_eq!(ids, [1], "nothing from the other database may reach the live one");
    }

    /// The other half of the same rule, on the side that runs first. The
    /// restore refuses a name that is not a plain file name, but only after
    /// `verify_unpacked` has already resolved and **hashed** it — and an
    /// absolute path makes `join` throw the base away. The digest of whatever
    /// that pointed at is then reported in the mismatch message, so a
    /// hand-delivered archive could be used to read the size and sha256 of any
    /// file this process can open.
    ///
    /// So the name is checked where it is first consumed. Asserted on the
    /// digest *not* appearing, which is the part that was leaking.
    #[tokio::test]
    async fn verification_does_not_read_a_file_the_manifest_points_outside_at() {
        let d = tempfile::tempdir().unwrap();
        let secret = d.path().join("host-file.db");
        std::fs::write(&secret, b"a secret the archive has no business hashing").unwrap();
        let secret_digest = digest_of(&secret).unwrap();

        // An unpacked archive whose manifest promises that absolute path, with
        // its digest supplied correctly so the mismatch branch is the one
        // reached.
        let dir = d.path().join("unpacked");
        std::fs::create_dir(&dir).unwrap();
        let manifest = BackupManifest {
            format: MANIFEST_FORMAT,
            created: Utc::now().to_rfc3339(),
            engine_version: "test".to_owned(),
            sqlite: MemberDigest {
                file: secret.to_str().unwrap().to_owned(),
                ..secret_digest.clone()
            },
            qdrant: None,
            qdrant_collection: None,
        };
        std::fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let err = verify_unpacked(&dir)
            .expect_err("a manifest naming a file outside the archive must be refused");
        let reported = format!("{err:#}");
        assert!(
            reported.contains("not a plain file name"),
            "the name must be refused as such: {reported}"
        );
        assert!(
            !reported.contains(&secret_digest.sha256),
            "the digest of a file outside the archive leaked into the report: {reported}"
        );
    }

    /// The rule itself, so it is stated once and tested once.
    #[test]
    fn a_plain_file_name_has_no_separator_no_parent_and_is_not_absolute() {
        for plain in ["backup.tar.gz", "rag_users.db", "a", "x.tar.gz"] {
            assert!(is_plain_file_name(plain), "{plain:?}");
        }
        for hostile in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "./x",
            "C:\\x",
        ] {
            assert!(!is_plain_file_name(hostile), "{hostile:?}");
        }
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

    /// The settings table (migration 0003) on the real schema, both ways: a
    /// backup taken before it existed restores into a binary that has it,
    /// leaving the table and the setting in it alone; a backup that has it
    /// brings its value back.
    #[tokio::test]
    async fn the_settings_table_restores_from_old_and_new_backups() {
        use crate::db::settings::{self, ANSWER_INSTRUCTIONS};
        let d = tempfile::tempdir().unwrap();
        let live = pool_at(&d.path().join("live.db")).await;
        crate::db::migrate(&live).await.unwrap();

        // A backup from before 0.2.0: the same schema without `settings`.
        let old_src = pool_at(&d.path().join("old-src.db")).await;
        crate::db::migrate(&old_src).await.unwrap();
        sqlx::query("DROP TABLE settings").execute(&old_src).await.unwrap();
        let old_backup = d.path().join("old-backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", old_backup.display()))
            .execute(&old_src).await.unwrap();

        settings::set(&live, ANSWER_INSTRUCTIONS, "Live text.", "admin").await.unwrap();
        let (tables, _) = restore_sqlite(&live, &old_backup).await.unwrap();
        assert!(!tables.contains(&"settings".to_string()), "{tables:?}");
        assert_eq!(
            settings::get(&live, ANSWER_INSTRUCTIONS).await.unwrap().as_deref(),
            Some("Live text."),
            "an archive without the table must leave it as it is"
        );

        // A backup taken with the setting in place, then changed: restoring
        // brings the archived value back.
        settings::set(&live, ANSWER_INSTRUCTIONS, "Archived text.", "admin").await.unwrap();
        let new_backup = d.path().join("new-backup.db");
        sqlx::query(&format!("VACUUM INTO '{}'", new_backup.display()))
            .execute(&live).await.unwrap();
        settings::set(&live, ANSWER_INSTRUCTIONS, "Changed later.", "admin").await.unwrap();
        let (tables, _) = restore_sqlite(&live, &new_backup).await.unwrap();
        assert!(tables.contains(&"settings".to_string()), "{tables:?}");
        assert_eq!(
            settings::get(&live, ANSWER_INSTRUCTIONS).await.unwrap().as_deref(),
            Some("Archived text.")
        );
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

    /// A restore that fails halfway must leave the live database and the
    /// pool behind it usable: the half-written copy is rolled back (so the
    /// live rows are still there) and the connection is handed back with no
    /// open transaction (so a retried restore can BEGIN, instead of failing
    /// with "cannot start a transaction within a transaction").
    ///
    /// Single-connection pool on purpose: with several connections the
    /// follow-up statements could land on a fresh one and pass even without
    /// the rollback, which would prove nothing.
    #[tokio::test]
    async fn a_failed_restore_rolls_back_and_leaves_the_pool_usable() {
        let d = tempfile::tempdir().unwrap();
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(d.path().join("live.db"))
            .create_if_missing(true);
        let live = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();

        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .execute(&live)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'live')")
            .execute(&live)
            .await
            .unwrap();

        // A backup missing the NOT NULL column: the shared-column copy
        // empties the live table first, then the INSERT..SELECT violates
        // the constraint halfway through.
        let bad = pool_at(&d.path().join("bad.db")).await;
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(&bad)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id) VALUES (1)")
            .execute(&bad)
            .await
            .unwrap();
        let bad_file = d.path().join("bad.db");
        bad.close().await;

        let err = restore_sqlite(&live, &bad_file).await.unwrap_err();
        assert!(
            err.to_string().contains("copying users"),
            "unexpected error: {err:#}"
        );

        // Rolled back, not half-emptied: the live row is still there.
        let name: String = sqlx::query_scalar("SELECT name FROM users WHERE id = 1")
            .fetch_one(&live)
            .await
            .unwrap();
        assert_eq!(name, "live");

        // The pool still accepts writes on the same connection.
        sqlx::query("INSERT INTO users (id, name) VALUES (2, 'after')")
            .execute(&live)
            .await
            .unwrap();

        // And a retried restore with a good archive works instead of
        // failing inside the leftover transaction.
        let good_file = d.path().join("good.db");
        sqlx::query(&format!("VACUUM INTO '{}'", good_file.display()))
            .execute(&live)
            .await
            .unwrap();
        let (tables, _) = restore_sqlite(&live, &good_file).await.unwrap();
        assert_eq!(tables, vec!["users".to_string()]);

        // Connection state was restored too.
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&live)
            .await
            .unwrap();
        assert_eq!(fk, 1);
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
        let archive = create_backup(&db, "", &url, coll, backup_dir.to_str().unwrap(), 0)
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
