//! Admin endpoints (admin role required):
//! POST   /api/admin/backup                  → trigger immediate backup
//! GET    /api/admin/backup/list             → list available archives
//! POST   /api/admin/backup/restore          → restore one of them
//! GET    /api/admin/qdrant/stats            → collection stats
//! GET    /api/admin/qdrant/documents        → unique documents in Qdrant
//! DELETE /api/admin/qdrant/document/{id}   → delete all vectors for a document
//! GET    /api/admin/sqlite/documents        → all rows (soft-deleted included)
//! GET    /api/admin/settings/answer-instructions → { instructions, is_default, default, max_chars }
//! PUT    /api/admin/settings/answer-instructions ← { instructions } → same shape (400 past max_chars)
//! DELETE /api/admin/settings/answer-instructions → back to the default instructions

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::auth::{jwt::Claims, rbac::Role};
use crate::backup::service;
use crate::db;
use crate::rag::prompt;
use crate::state::AppState;

/// A 4xx tells the caller what they got wrong, so its message travels.
/// A 5xx does not: `msg` is an anyhow chain carrying whatever context the
/// failure picked up on the way out — filesystem paths, the Qdrant URL, SQL
/// text, the body of an eullm reply — and handing that to an unauthenticated
/// caller is free reconnaissance. The detail goes to the log, where it is
/// actually useful, and the response says only that something broke.
fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    if status.is_server_error() {
        tracing::error!(status = %status, detail = %msg, "request failed");
        return (status, Json(json!({ "error": "internal server error" }))).into_response();
    }
    (status, Json(json!({ "error": msg.to_string() }))).into_response()
}

fn require_admin(claims: &Claims) -> Option<Response> {
    if claims.role != Role::Admin {
        Some(err(StatusCode::FORBIDDEN, "admin role required"))
    } else {
        None
    }
}

// ── POST /api/admin/backup ────────────────────────────────────────────────────

pub async fn trigger_backup(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) {
        return r;
    }

    // Extract the filesystem path from the sqlite:// URL
    let db_path = state
        .settings
        .database
        .url
        .trim_start_matches("sqlite://")
        .to_owned();

    match service::create_backup(
        &state.db,
        &db_path,
        &state.settings.qdrant.url,
        &state.settings.qdrant.collection,
        &state.settings.backup.dir,
        state.settings.backup.retain_last,
    )
    .await
    {
        Ok(path) => Json(json!({
            "ok": true,
            "archive": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ── GET /api/admin/backup/list ────────────────────────────────────────────────

pub async fn list_backups(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) {
        return r;
    }
    let archives = service::list_backups(&state.settings.backup.dir).await;
    Json(json!({ "backups": archives })).into_response()
}

// ── POST /api/admin/backup/restore ────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct RestoreRequest {
    /// File name of an archive in the backup directory, as returned by
    /// GET /api/admin/backup/list.
    archive: String,
}

/// Restore a backup over the running installation.
///
/// Destructive by nature: the contents of the Qdrant collection and of every
/// table also present in the archive are replaced, not merged. It is admin-only
/// for that reason.
pub async fn restore_backup(
    State(state): State<AppState>,
    claims: Claims,
    Json(req): Json<RestoreRequest>,
) -> Response {
    if let Some(r) = require_admin(&claims) {
        return r;
    }

    tracing::warn!(
        archive = %req.archive,
        user = %claims.username,
        "restore requested: the collection and the database are about to be replaced"
    );

    match service::restore_backup(
        &state.db,
        &state.settings.qdrant.url,
        &state.settings.qdrant.collection,
        &state.settings.backup.dir,
        &req.archive,
    )
    .await
    {
        Ok(report) => Json(json!({ "ok": true, "restored": report })).into_response(),
        // A bad archive - the name, or the archive itself - is the caller's
        // mistake, not a server fault. Every other failure happened while
        // restoring and is ours.
        Err(e) if is_bad_request(&e) => err(StatusCode::BAD_REQUEST, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// Whether the restore failed because of the archive or the name it was given,
/// rather than because of us.
///
/// The archive's own failures are the caller's to fix - wrong name, damaged or
/// truncated tarball, a digest that does not match, a member missing, a
/// manifest naming something outside the archive, a snapshot belonging to a
/// collection this installation is not configured for. Reporting those as 5xx
/// was not merely a wrong status: `err` deliberately withholds a 5xx's message
/// from the caller, so an admin who picked the wrong archive, or restored one
/// taken from another collection, was told "internal server error" and nothing
/// about which archive or why.
///
/// Matched against the whole chain, not `e.to_string()`: that is only the
/// outermost context, so any `.context()` added above one of these in the
/// service would silently demote a 4xx to a 5xx with the message hidden. The
/// test below holds every refusal the service can produce here, so a new one
/// has to be classified deliberately rather than by omission.
fn is_bad_request(e: &anyhow::Error) -> bool {
    let msg = format!("{e:#}");
    const CALLER_FAULT: &[&str] = &[
        // resolve_archive
        "invalid archive name",
        "not a backup archive",
        "archive not found",
        // unpack_tar_gz
        "archive entry escapes the destination",
        "archive entry is not a regular file or directory",
        "the archive's gzip stream",
        // verify_unpacked
        "parsing backup.json",
        "this archive is in backup format",
        "not a plain file name",
        "the archive promises",
        "does not match the archive's own manifest",
        // restore_backup
        "the archive contains no",
        "holds the Qdrant collection",
    ];
    CALLER_FAULT.iter().any(|reason| msg.contains(reason))
}

// ── GET /api/admin/qdrant/stats ───────────────────────────────────────────────

/// What to do with a Qdrant collection-info reply.
///
/// Split out from the handler so the rule is testable without a Qdrant: a
/// non-success status is a 502 carrying the status and the body, and only a
/// success is handed to the browser as the collection's state.
fn collection_info_response(status: reqwest::StatusCode, body: String) -> Response {
    if status.is_success() {
        return match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => Json(value).into_response(),
            Err(e) => err(StatusCode::BAD_GATEWAY, e),
        };
    }
    err(
        StatusCode::BAD_GATEWAY,
        format!("Qdrant refused the collection info ({status}): {body}"),
    )
}

pub async fn qdrant_stats(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    let url = format!("{}/collections/{}", state.settings.qdrant.url, state.settings.qdrant.collection);
    match reqwest::get(&url).await {
        // The status is checked before the body is passed on. This handler is
        // the one place a Qdrant reply reaches the browser verbatim, so a 404
        // ("Not found: collection …") or a 401 was rendered by the admin panel
        // as if it were the collection's real state: an error object where the
        // tab expects result.points_count, which reads as "0 points" for a
        // collection that is full. The rest of this file reports the status and
        // the body; this now does too.
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            collection_info_response(status, body)
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

// ── GET /api/admin/qdrant/documents ──────────────────────────────────────────

pub async fn qdrant_documents(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    let client = reqwest::Client::new();
    match scroll_all_documents(&client, &state.settings.qdrant.url, &state.settings.qdrant.collection).await {
        Ok(list) => Json(json!({ "documents": list })).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

/// How many points one scroll request asks for.
const SCROLL_PAGE: usize = 1000;

/// Ceiling on the pages one call will fetch, so a misbehaving or hostile
/// endpoint that always reports another page cannot spin here forever.
const SCROLL_MAX_PAGES: usize = 1000;

/// Every document in the collection, with the number of chunks each has.
///
/// The scroll is followed to the end. Asking once with a limit and taking what
/// comes back is the shape this used to have, and it silently truncates: the
/// limit is in **points**, and a document is a few dozen of those, so a
/// collection of any size stopped at the first page and the admin's sync check
/// then reported every document past it as "in SQLite but not in Qdrant" -
/// documents that are entirely present and entirely fine, listed for deletion.
async fn scroll_all_documents(
    client: &reqwest::Client,
    qdrant_url: &str,
    collection: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let url = format!("{qdrant_url}/collections/{collection}/points/scroll");
    let mut docs: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    // A point id. This project's are UUID strings, and the scroll offset is
    // whatever the previous page's next_page_offset was, so it is passed back
    // exactly as Qdrant gave it rather than read as a number.
    let mut offset: Option<serde_json::Value> = None;

    for _ in 0..SCROLL_MAX_PAGES {
        let mut body = json!({
            "limit": SCROLL_PAGE,
            "with_payload": true,
            "with_vector": false,
        });
        if let Some(at) = &offset {
            body["offset"] = at.clone();
        }
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("scrolling the vectors of {collection}"))?;
        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            anyhow::bail!("Qdrant refused the scroll of {collection} ({status}): {detail}");
        }
        let raw: serde_json::Value = resp
            .json()
            .await
            .with_context(|| format!("reading the scroll of {collection}"))?;

        // A page short of the limit is the last one. Qdrant also hands back
        // `next_page_offset`, and that is the authoritative answer, so it is
        // preferred and the length is only the fallback for an older build that
        // omits it.
        let result = raw.get("result");
        let points = result
            .and_then(|r| r.get("points"))
            .and_then(|p| p.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        let next = result
            .and_then(|r| r.get("next_page_offset"))
            .filter(|v| !v.is_null())
            .cloned();
        let last = next.is_none() && points < SCROLL_PAGE;

        // Group by document_id from the payloads.
        if let Some(points) = result
            .and_then(|r| r.get("points"))
            .and_then(|p| p.as_array())
        {
            for point in points {
                let Some(payload) = point.get("payload") else {
                    continue;
                };
                let doc_id = payload.get("document_id").and_then(|v| v.as_str()).unwrap_or("");
                if doc_id.is_empty() { continue; }
                let entry = docs.entry(doc_id.to_owned()).or_insert_with(|| json!({
                    "document_id": doc_id,
                    "filename": payload.get("filename").and_then(|v| v.as_str()).unwrap_or(""),
                    "upload_date": payload.get("upload_date").and_then(|v| v.as_str()).unwrap_or(""),
                    "chunk_count": 0_i64,
                }));
                if let Some(n) = entry.get_mut("chunk_count").and_then(|v| v.as_i64()) {
                    *entry.get_mut("chunk_count").unwrap() = json!(n + 1);
                }
            }
        }

        match next {
            Some(at) if !last => offset = Some(at),
            _ => break,
        }
    }

    let mut list: Vec<serde_json::Value> = docs.into_values().collect();
    list.sort_by(|a, b| {
        let da = a.get("upload_date").and_then(|v| v.as_str()).unwrap_or("");
        let db = b.get("upload_date").and_then(|v| v.as_str()).unwrap_or("");
        db.cmp(da)
    });
    Ok(list)
}

// ── DELETE /api/admin/qdrant/document/{id} ────────────────────────────────────

pub async fn qdrant_delete_document(
    State(state): State<AppState>,
    claims: Claims,
    Path(document_id): Path<String>,
) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    // INVARIANT: use the SINGLE entry point shared with the user-facing
    // delete — Qdrant AND SQLite together, Qdrant first. This handler used to
    // delete from Qdrant ONLY, over REST, leaving the SQLite row "active" with
    // no vectors: an orphan, and exactly the second entry point the invariant
    // exists to forbid.
    match crate::api::documents::purge_document(&state, &document_id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "document not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ── GET /api/admin/sqlite/documents ──────────────────────────────────────────

pub async fn sqlite_documents(State(state): State<AppState>, claims: Claims) -> Response {
    if let Some(r) = require_admin(&claims) { return r; }
    match db::documents::list_all(&state.db).await {
        Ok(docs) => Json(json!({ "documents": docs })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ── /api/admin/settings/answer-instructions ──────────────────────────────────
//
// The instructions the answering model is given (rag::prompt). The handlers
// only unwrap the request: the work is in the functions below them, which
// take the pool and the caller, so it is tested without a running server.

#[derive(serde::Deserialize)]
pub struct AnswerInstructionsBody {
    instructions: String,
}

pub async fn get_answer_instructions(State(state): State<AppState>, claims: Claims) -> Response {
    read_answer_instructions(&state.db, &claims).await
}

pub async fn put_answer_instructions(
    State(state): State<AppState>,
    claims: Claims,
    Json(body): Json<AnswerInstructionsBody>,
) -> Response {
    save_answer_instructions(&state.db, &claims, &body.instructions).await
}

pub async fn delete_answer_instructions(State(state): State<AppState>, claims: Claims) -> Response {
    reset_answer_instructions(&state.db, &claims).await
}

async fn read_answer_instructions(pool: &sqlx::SqlitePool, claims: &Claims) -> Response {
    if let Some(r) = require_admin(claims) {
        return r;
    }
    answer_instructions_state(pool).await
}

async fn save_answer_instructions(pool: &sqlx::SqlitePool, claims: &Claims, text: &str) -> Response {
    if let Some(r) = require_admin(claims) {
        return r;
    }
    let chars = text.chars().count();
    if chars > prompt::MAX_INSTRUCTIONS_CHARS {
        return err(
            StatusCode::BAD_REQUEST,
            format!(
                "the instructions are {chars} characters long; the limit is {}",
                prompt::MAX_INSTRUCTIONS_CHARS
            ),
        );
    }
    // Blank is not a setting: it would be the default anyway, so the row is
    // removed and a later release's default reaches this install too.
    let result = if text.trim().is_empty() {
        db::settings::reset(pool, db::settings::ANSWER_INSTRUCTIONS).await.map(|_| ())
    } else {
        db::settings::set(pool, db::settings::ANSWER_INSTRUCTIONS, text, &claims.username).await
    };
    if let Err(e) = result {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("saving the answer instructions: {e:#}"));
    }
    tracing::info!(admin = %claims.username, chars, "answer instructions changed");
    answer_instructions_state(pool).await
}

async fn reset_answer_instructions(pool: &sqlx::SqlitePool, claims: &Claims) -> Response {
    if let Some(r) = require_admin(claims) {
        return r;
    }
    if let Err(e) = db::settings::reset(pool, db::settings::ANSWER_INSTRUCTIONS).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("resetting the answer instructions: {e:#}"));
    }
    tracing::info!(admin = %claims.username, "answer instructions reset to the default");
    answer_instructions_state(pool).await
}

/// What GET answers, and PUT and DELETE after their change: the instructions
/// in effect, whether they are the default, the default itself (for the
/// panel's reset), and the limit (for its counter).
async fn answer_instructions_state(pool: &sqlx::SqlitePool) -> Response {
    let saved = match db::settings::get(pool, db::settings::ANSWER_INSTRUCTIONS).await {
        Ok(saved) => saved,
        Err(e) => {
            return err(StatusCode::INTERNAL_SERVER_ERROR, format!("reading the answer instructions: {e:#}"))
        }
    };
    let is_default = saved.as_deref().is_none_or(|t| t.trim().is_empty());
    Json(json!({
        "instructions": saved.filter(|_| !is_default).unwrap_or_else(|| prompt::DEFAULT_INSTRUCTIONS.to_owned()),
        "is_default": is_default,
        "default": prompt::DEFAULT_INSTRUCTIONS,
        "max_chars": prompt::MAX_INSTRUCTIONS_CHARS,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::collection_info_response;
    use super::is_bad_request;
    use super::scroll_all_documents;

    /// The admin panel's sync check asks Qdrant which documents exist and
    /// reports any that SQLite has and Qdrant does not. That answer came from
    /// a single scroll with a fixed limit, so on any collection past it the
    /// check listed perfectly healthy documents as missing - and the panel
    /// offers to delete them.
    ///
    /// The pages are followed here: the fake answers with one page, then a
    /// second, then stops.
    #[tokio::test]
    async fn the_scroll_is_followed_until_qdrant_says_there_is_no_more() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    let read = stream.read(&mut buf).await.unwrap_or(0);
                    // Which page is being asked for is decided by the request
                    // body carrying an offset.
                    let asked_for_second = read > 0
                        && String::from_utf8_lossy(&buf[..read]).contains("\"offset\"");
                    let page: serde_json::Value = if asked_for_second {
                        serde_json::json!({
                            "result": {
                                "points": [
                                    { "payload": { "document_id": "doc-b", "filename": "b.pdf",
                                                   "upload_date": "2026-01-02T00:00:00Z" } },
                                    { "payload": { "document_id": "doc-c", "filename": "c.pdf",
                                                   "upload_date": "2026-01-03T00:00:00Z" } },
                                ],
                                "next_page_offset": null,
                            }
                        })
                    } else {
                        serde_json::json!({
                            "result": {
                                "points": [
                                    { "payload": { "document_id": "doc-a", "filename": "a.pdf",
                                                   "upload_date": "2026-01-01T00:00:00Z" } },
                                    { "payload": { "document_id": "doc-b", "filename": "b.pdf",
                                                   "upload_date": "2026-01-02T00:00:00Z" } },
                                ],
                                // Point ids here are UUIDs, so the offset is a
                                // string, as Qdrant returns it for them.
                                "next_page_offset": "3f2b8c1e-7d4a-4e9b-9a6f-2c1d0e5b7a91",
                            }
                        })
                    };
                    let body = page.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        let list = scroll_all_documents(
            &reqwest::Client::new(),
            &format!("http://{addr}"),
            "rag_documents",
        )
        .await
        .expect("both pages");
        let ids: Vec<&str> = list
            .iter()
            .map(|d| d["document_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["doc-c", "doc-b", "doc-a"],
            "newest first, and doc-c is only on the second page"
        );
        let chunks = |id: &str| {
            list.iter()
                .find(|d| d["document_id"] == id)
                .unwrap()["chunk_count"]
                .as_i64()
                .unwrap()
        };
        assert_eq!(chunks("doc-b"), 2, "a document spanning two pages is counted once per chunk");
    }

    /// Qdrant's error objects are valid JSON, so parsing one used to succeed and
    /// the panel was handed `{"status": {"error": "Not found: collection …"}}`
    /// where it looks for `result.points_count` — which reads as "0 points" and
    /// "0 vectors". A full collection, a wrong api-key and a typo in
    /// QDRANT__COLLECTION all looked like an empty one.
    ///
    /// The distinction that matters is 200 against not-200: a refusal must not
    /// be rendered as the collection's state. The status and body go to the log
    /// rather than to the caller, which is this codebase's standing rule for
    /// 5xx — the point is that the panel shows a failure, not a plausible lie.
    #[tokio::test]
    async fn a_refused_collection_info_is_never_passed_through_as_state() {
        let error_body =
            r#"{"status":{"error":"Not found: collection `rag_documents` doesn't exist!","status":"error"}}"#
                .to_owned();
        let refused = collection_info_response(reqwest::StatusCode::NOT_FOUND, error_body);
        assert_eq!(refused.status(), axum::http::StatusCode::BAD_GATEWAY);
        let text = axum::body::to_bytes(refused.into_body(), 64 * 1024).await.unwrap();
        let reported = String::from_utf8_lossy(&text);
        assert!(
            !reported.contains("doesn't exist"),
            "Qdrant's error object must not reach the browser as if it were state: {reported}"
        );

        // And a real reply, with the same shape Qdrant really returns, still
        // goes through untouched.
        let ok = r#"{"result":{"status":"green","points_count":42,"vectors_count":42}}"#;
        let passed = collection_info_response(reqwest::StatusCode::OK, ok.to_owned());
        assert_eq!(passed.status(), axum::http::StatusCode::OK);
        let text = axum::body::to_bytes(passed.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&text).contains("\"points_count\":42"));
    }

    /// A 200 that is not the expected shape is an error, not a blank tab that
    /// reads as "no vectors".
    #[tokio::test]
    async fn a_successful_but_unparsable_reply_is_a_502() {
        let response = collection_info_response(reqwest::StatusCode::OK, "not json".to_owned());
        assert_eq!(response.status(), axum::http::StatusCode::BAD_GATEWAY);
    }

    fn bad(msg: &str) -> anyhow::Error {
        anyhow::anyhow!("{msg}")
    }

    /// Every refusal backup::service can produce about the archive or the name
    /// it was given, held here so a drift misclassifying a caller error as a
    /// 500 shows up as a failing test rather than as an admin told
    /// "internal server error".
    #[test]
    fn caller_errors_are_recognised() {
        for msg in [
            // resolve_archive
            "invalid archive name: \"../x.tar.gz\"",
            "not a backup archive: \"rag_users.db\"",
            "archive not found: absent.tar.gz",
            // unpack_tar_gz
            "archive entry escapes the destination: x",
            "archive entry is not a regular file or directory: link",
            "the archive's gzip stream is truncated or its checksum does not match",
            // verify_unpacked
            "parsing backup.json: expected value at line 1 column 1",
            "this archive is in backup format 9 and was written by a newer engine (x)",
            "the archive's manifest names \"/etc/passwd\", which is not a plain file name",
            "the archive promises rag_users.db but does not contain it",
            "rag_users.db does not match the archive's own manifest - the backup is damaged",
            // restore_backup
            "the archive contains no rag_users.db: nothing was restored",
            "this archive holds the Qdrant collection \"old\", but this installation is \
             configured for \"new\"",
        ] {
            assert!(is_bad_request(&bad(msg)), "msg={msg:?}");
        }
    }

    /// And ours, which must keep their 5xx - and with it the withheld message.
    #[test]
    fn server_faults_are_not_recognised() {
        for msg in [
            "the Qdrant snapshot could not be verified, backup aborted",
            "the Qdrant snapshot is corrupt: expected sha256 aa, got bb",
            "internal server error",
            "",
        ] {
            assert!(!is_bad_request(&bad(msg)), "msg={msg:?}");
        }
    }

    /// The classification reads the whole chain, so a `.context()` added above
    /// one of these in the service cannot quietly demote a 4xx into a 5xx with
    /// the message withheld.
    #[test]
    fn a_wrapped_caller_error_is_still_a_caller_error() {
        let wrapped = bad("invalid archive name: \"x\"").context("restoring backup");
        assert!(is_bad_request(&wrapped), "{wrapped:#}");
    }

    // ── answer instructions ──────────────────────────────────────────────

    use super::*;
    use crate::auth::{jwt::Claims, rbac::Role};
    use axum::{http::StatusCode, response::Response};

    async fn settings_pool(path: &std::path::Path) -> sqlx::SqlitePool {
        let url = format!("sqlite://{}", path.display());
        let pool = crate::db::connect(&url).await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    fn caller(role: Role) -> Claims {
        Claims { user_id: 1, username: "someone".into(), role, exp: u64::MAX }
    }

    async fn body_of(r: Response) -> (StatusCode, serde_json::Value) {
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn only_an_admin_reads_or_changes_the_answer_instructions() {
        let d = tempfile::tempdir().unwrap();
        let pool = settings_pool(&d.path().join("a.db")).await;
        for role in [Role::User, Role::SuperUser] {
            let who = caller(role);
            assert_eq!(read_answer_instructions(&pool, &who).await.status(), StatusCode::FORBIDDEN);
            assert_eq!(save_answer_instructions(&pool, &who, "x").await.status(), StatusCode::FORBIDDEN);
            assert_eq!(reset_answer_instructions(&pool, &who).await.status(), StatusCode::FORBIDDEN);
        }
        assert_eq!(
            db::settings::get(&pool, db::settings::ANSWER_INSTRUCTIONS).await.unwrap(),
            None,
            "a refused change must not be saved"
        );
    }

    #[tokio::test]
    async fn instructions_past_the_limit_are_refused_and_the_limit_itself_is_not() {
        let d = tempfile::tempdir().unwrap();
        let pool = settings_pool(&d.path().join("a.db")).await;
        let admin = caller(Role::Admin);
        let too_long: String = "à".repeat(prompt::MAX_INSTRUCTIONS_CHARS + 1);
        let (status, body) = body_of(save_answer_instructions(&pool, &admin, &too_long).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("8000"), "{body}");
        // Characters, not bytes: 8000 accented letters are 16000 bytes and fit.
        let at_limit: String = "à".repeat(prompt::MAX_INSTRUCTIONS_CHARS);
        assert_eq!(save_answer_instructions(&pool, &admin, &at_limit).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn saved_instructions_survive_a_restart_and_delete_brings_the_default_back() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("a.db");
        let admin = caller(Role::Admin);
        {
            let pool = settings_pool(&path).await;
            let (_, before) = body_of(read_answer_instructions(&pool, &admin).await).await;
            assert_eq!(before["is_default"], true);
            assert_eq!(before["instructions"], prompt::DEFAULT_INSTRUCTIONS);
            assert_eq!(before["default"], prompt::DEFAULT_INSTRUCTIONS);
            assert_eq!(before["max_chars"], prompt::MAX_INSTRUCTIONS_CHARS);

            let (status, saved) = body_of(save_answer_instructions(&pool, &admin, "Answer in one line.").await).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(saved["is_default"], false);
            assert_eq!(saved["instructions"], "Answer in one line.");
            pool.close().await;
        }
        let pool = settings_pool(&path).await;
        let (_, after) = body_of(read_answer_instructions(&pool, &admin).await).await;
        assert_eq!(after["instructions"], "Answer in one line.", "the restart lost it");
        assert_eq!(after["is_default"], false);

        let (status, reset) = body_of(reset_answer_instructions(&pool, &admin).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reset["is_default"], true);
        assert_eq!(reset["instructions"], prompt::DEFAULT_INSTRUCTIONS);
        assert_eq!(db::settings::get(&pool, db::settings::ANSWER_INSTRUCTIONS).await.unwrap(), None);
    }

    #[tokio::test]
    async fn saving_blank_instructions_is_the_default() {
        let d = tempfile::tempdir().unwrap();
        let pool = settings_pool(&d.path().join("a.db")).await;
        let admin = caller(Role::Admin);
        save_answer_instructions(&pool, &admin, "Custom.").await;
        let (status, body) = body_of(save_answer_instructions(&pool, &admin, "  \n\t ").await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["is_default"], true);
        assert_eq!(db::settings::get(&pool, db::settings::ANSWER_INSTRUCTIONS).await.unwrap(), None);
    }
}
