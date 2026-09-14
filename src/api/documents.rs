//! Document management endpoints:
//! GET    /api/documents
//! POST   /api/documents/upload   — multipart/form-data, field "file"
//! DELETE /api/documents/{id}

use anyhow::Context;
use axum::{
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::auth::jwt::Claims;
use crate::bench;
use crate::db;
use crate::rag::vector_store::ChunkPayload;
use crate::documents::parser;
use crate::rag::chunker;
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

// ── GET /api/documents ────────────────────────────────────────────────────────

pub async fn list(State(state): State<AppState>, _claims: Claims) -> Response {
    match db::documents::list_active(&state.db).await {
        Ok(docs) => Json(json!({ "documents": docs })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ── POST /api/documents/upload ────────────────────────────────────────────────

pub async fn upload(
    State(state): State<AppState>,
    claims: Claims,
    multipart: Multipart,
) -> Response {
    // RBAC (parity with the Python require_upload_permission → admin|super_user).
    if !claims.role.can_upload() {
        return err(StatusCode::FORBIDDEN, "insufficient permissions to upload documents");
    }
    // The guard does two things, both for the WHOLE window of
    // unload → extract/chunk/embed → reload rather than just the heavy part.
    // It keeps active_ingestions > 0 (see AppState::ingestion_blocks_queries):
    // dropped before the reload, a concurrent query would load eullm again on
    // its own while the embedding model is still using the freed VRAM. And it
    // holds the single ingestion permit, so a second upload waits here instead
    // of reloading the chat model into VRAM that this one is still embedding
    // in — see IngestionGuard::start.
    let _ingestion_guard =
        match crate::state::IngestionGuard::start(&state.active_ingestions, &state.ingestion_slot)
            .await
        {
            Ok(guard) => guard,
            // A closed semaphore is our fault, not the caller's: err() logs
            // the detail and the response stays a bare 500.
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
    let unload_enabled = state.settings.eullm.unload_during_ingestion;
    let ingestion_embedding = state.settings.embeddings.ingestion_embedding;
    let candle_gpu = ingestion_embedding == crate::config::IngestionEmbedding::CandleGpu;
    let via_eullm = ingestion_embedding == crate::config::IngestionEmbedding::Eullm;

    // Order: free the VRAM (eullm) BEFORE taking it (bge-m3) — and on the
    // way out the mirror image: bge-m3 leaves the VRAM BEFORE eullm reclaims
    // it. eullm sizes its offload from the free VRAM it reads at load time,
    // so reloading it while bge-m3 is still resident would size it against
    // memory that is about to be released. Only relevant to CandleGpu: with
    // Eullm, eullm evicts (or not) its own chat model by itself the moment
    // process_upload() asks it to embed — no manual unload from us either
    // way, config::validate_ingestion_embedding lets unload_during_ingestion
    // be true here too but it would just be a redundant no-op unload.
    if unload_enabled {
        if let Err(e) = state.eullm.unload().await {
            tracing::error!(error = %e, "eullm: unload before ingestion failed, continuing anyway (no VRAM freed)");
        }
    }
    if candle_gpu {
        if let Err(e) = swap_embeddings_blocking(&state, true).await {
            tracing::error!(error = %e, "embedding: swap to GPU failed, ingestion will use the CPU (much slower)");
        }
    }

    let response = process_upload(&state, multipart).await;

    if candle_gpu {
        if let Err(e) = swap_embeddings_blocking(&state, false).await {
            tracing::error!(error = %e, "embedding: swap to CPU failed — bge-m3 may still be in VRAM, check manually");
        }
    }
    // Bring the chat model back promptly instead of leaving its reload for
    // whichever user query happens to arrive first. Fires for Eullm too,
    // not just the manual-unload case: asking eullm for bge-m3 may itself
    // have evicted the chat model (eullm's decision, see
    // config::IngestionEmbedding::Eullm), and reload() is a harmless no-op
    // if it turns out eullm never evicted it at all.
    if unload_enabled || via_eullm {
        if let Err(e) = state.eullm.reload().await {
            tracing::error!(error = %e, "eullm: reload after ingestion failed — the model may not be resident in VRAM, check manually");
        }
    }

    response
}

/// Swaps the embedding model's device off the async executor (it blocks:
/// mmap plus a weight copy). `to_gpu=true` moves it to the GPU at the start
/// of an ingestion, `false` back to the CPU at the end — see
/// config::IngestionEmbedding::CandleGpu.
async fn swap_embeddings_blocking(state: &AppState, to_gpu: bool) -> anyhow::Result<()> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        if to_gpu { state.swap_embeddings_to_gpu() } else { state.swap_embeddings_to_cpu() }
    })
    .await
    .context("join swap task")?
}

async fn process_upload(state: &AppState, mut multipart: Multipart) -> Response {
    // 1. Stream the "file" field straight to a temp file, hashing
    // incrementally and rejecting as soon as the configured cap is
    // exceeded — the body is never held in RAM (issue #43). The
    // `DefaultBodyLimit` in api::router stays as a backstop behind this
    // early rejection.
    let max_bytes = state.settings.storage.max_upload_bytes();
    let mut filename = String::new();
    let mut ext = String::new();
    let mut received: Option<(u64, String)> = None;
    let mut tmp_path = std::path::PathBuf::new();

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name().unwrap_or("") == "file" {
                    filename = field.file_name().unwrap_or("upload").to_string();
                    // The parser dispatches on extension, so the temp file
                    // keeps it (unchanged behaviour, just computed earlier).
                    ext = std::path::Path::new(&filename)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    // Refuse a format the parser cannot read BEFORE writing
                    // any of it. Until now a .pptx was streamed to disk in
                    // full — up to the configured limit — and only then
                    // rejected by extract_text at step 2.
                    if !parser::is_supported_extension(&ext) {
                        return err(
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            format!(
                                "unsupported format: .{ext} — accepted: {}",
                                parser::SUPPORTED_EXTENSIONS.join(", ")
                            ),
                        );
                    }
                    tmp_path = match upload_tmp_path(state, &ext).await {
                        Ok(p) => p,
                        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
                    };
                    futures_util::pin_mut!(field);
                    match stream_upload_to_file(field, &tmp_path, max_bytes).await {
                        Ok((len, sha)) => {
                            if len == 0 {
                                let _ = tokio::fs::remove_file(&tmp_path).await;
                                return err(
                                    StatusCode::BAD_REQUEST,
                                    "no file in multipart request (field name: \"file\")",
                                );
                            }
                            received = Some((len, sha));
                            break;
                        }
                        Err(ReceiveError::TooLarge) => {
                            return err(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                format!(
                                    "upload exceeds the {} MB limit",
                                    state.settings.storage.max_upload_mb
                                ),
                            );
                        }
                        Err(ReceiveError::Read(e)) => return err(StatusCode::BAD_REQUEST, e),
                        Err(ReceiveError::Write(e)) => {
                            return err(StatusCode::INTERNAL_SERVER_ERROR, e)
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => return err(StatusCode::BAD_REQUEST, e),
        }
    }

    let Some((_, content_sha256)) = received else {
        return err(
            StatusCode::BAD_REQUEST,
            "no file in multipart request (field name: \"file\")",
        );
    };
    // From here on the temp file exists: own it till the end of the
    // function, so every early return below removes it (the receive step
    // already removed its own partials on the paths above).
    let _tmp_guard = RemoveOnDrop(tmp_path.clone());

    // Content identity for provenance_id (see rag::chunker::provenance_id):
    // anchored to the uploaded bytes, NOT to document_id below (a fresh UUID
    // every upload) — re-ingesting this same file must yield the same hash,
    // and therefore the same provenance_id per chunk, given an unchanged
    // chunking configuration. The digest is computed incrementally during
    // reception above, so it is identical to a one-shot hash of the body.

    // 2. Extract text (sync, possibly heavy — run off the async executor).
    // Timed even when --bench-live is off: an Instant::now() costs nothing
    // worth conditionalising.
    let extract_start = std::time::Instant::now();
    let extracted = match tokio::task::spawn_blocking({
        let tmp = tmp_path.clone();
        let data_dir = state.settings.data.data_path();
        move || parser::extract_text(&tmp, &data_dir)
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("parse task: {e}")),
    };
    let extract_time = extract_start.elapsed();
    let parser::ExtractedText { text, page_count, pages } = extracted;

    // 3. Chunk
    let chunk_start = std::time::Instant::now();
    let chunks = chunker::split_text(&text);
    let chunk_time = chunk_start.elapsed();
    if chunks.is_empty() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no text could be extracted from this document",
        );
    }

    // 3b. Enrich each chunk before it is embedded/stored — Community's own
    // default prepends the nearest preceding structural heading ("Article
    // 99", "Chapter XII", ...) to any chunk that doesn't already start with
    // it (see extensions::ingestion::DefaultChunkEnricher, wrapping
    // chunker::inject_heading_context); a Pro build can register a
    // different enricher (e.g. Contextual Retrieval) here instead, per
    // extensions::ChunkEnricher. Either way, `chunks[i].start_byte`/
    // `end_byte` (used below for page lookups and citation spans) still
    // point at the real source location, untouched by whatever enrichment
    // ran.
    let chunk_texts = match state.extensions.chunk_enricher.enrich(&text, &chunks).await {
        Ok(texts) => texts,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("chunk enrichment: {e}")),
    };

    // 4. Embed. Candle is CPU/GPU-bound (spawn_blocking); eullm is an HTTP
    // call (.await directly) — see config::IngestionEmbedding::Eullm.
    let embed_start = std::time::Instant::now();
    let embeddings = if state.settings.embeddings.ingestion_embedding
        == crate::config::IngestionEmbedding::Eullm
    {
        let refs: Vec<&str> = chunk_texts.iter().map(|s| s.as_str()).collect();
        match state.eullm.embed_texts(crate::config::EULLM_EMBEDDING_MODEL, &refs, None).await {
            Ok(e) => e,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("eullm embedding: {e}")),
        }
    } else {
        let embeddings_svc = match state.embeddings.clone() {
            Some(svc) => svc,
            None => {
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Candle embeddings not loaded (ingestion_embedding=eullm?)",
                )
            }
        };
        let chunk_strs = chunk_texts.clone();
        match tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = chunk_strs.iter().map(|s| s.as_str()).collect();
            let guard = embeddings_svc
                .read()
                .map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
            guard.embed_texts(&refs)
        })
        .await
        {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("embedding: {e}")),
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("embed task: {e}")),
        }
    };
    let embed_time = embed_start.elapsed();

    // 5. Build Qdrant payloads and upsert
    let document_id = uuid::Uuid::new_v4().to_string();
    let upload_date = Utc::now().to_rfc3339();

    let payloads: Vec<ChunkPayload> = chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let (page_start, page_end) =
                parser::pages_for_range(&pages, chunk.start_byte, chunk.end_byte)
                    .map_or((None, None), |(s, e)| (Some(s), Some(e)));
            ChunkPayload {
                document_id: document_id.clone(),
                chunk_index: i,
                filename: filename.clone(),
                upload_date: upload_date.clone(),
                // text is ALWAYS the real, unmodified chunk — citations and
                // source highlighting must never show enricher output (see
                // ChunkPayload::retrieval_text's doc comment). The embedding
                // vector above is still computed from chunk_texts (enriched).
                text: chunk.text.clone(),
                chunk_size: chunk.text.len(),
                document_type: ext.clone(),
                structured_fields: None,
                source_start_byte: Some(chunk.start_byte),
                source_end_byte: Some(chunk.end_byte),
                page_start,
                page_end,
                provenance_id: Some(chunker::provenance_id(
                    &content_sha256,
                    i,
                    parser::EXTRACTION_CONFIG_VERSION,
                )),
                retrieval_text: (chunk_texts[i] != chunk.text).then(|| chunk_texts[i].clone()),
            }
        })
        .collect();

    let upsert_start = std::time::Instant::now();
    if let Err(e) = state.qdrant.upsert(&embeddings, &payloads).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("qdrant upsert: {e}"));
    }
    let upsert_time = upsert_start.elapsed();

    // --bench-live: record this real ingestion (see bench::LiveRecorder).
    if let Some(rec) = &state.live_bench {
        rec.record_ingestion(
            filename.clone(),
            bench::IngestionResult {
                document_id: document_id.clone(),
                stages: vec![
                    bench::StageTiming { name: "Estrazione testo", duration: extract_time },
                    bench::StageTiming { name: "Chunking", duration: chunk_time },
                    bench::StageTiming { name: "Embedding", duration: embed_time },
                    bench::StageTiming { name: "Upsert Qdrant", duration: upsert_time },
                ],
                page_count,
                word_count: text.split_whitespace().count(),
                char_count: text.chars().count(),
                chunk_count: chunks.len(),
            },
        );
    }

    // 6. Persist metadata in SQLite
    if let Err(e) =
        db::documents::insert(&state.db, &document_id, &filename, page_count, &ext, chunks.len())
            .await
    {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("db insert: {e}"));
    }

    // 7. Save original file for later download (best-effort) — a copy of
    // the temp file received in step 1, which the guard removes.
    let orig_path = state.storage.path_for(&document_id, &filename);
    if let Some(parent) = orig_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    // tokio::fs, not std::fs: this copies the whole upload, up to the
    // configured limit, and on the async executor that stalls every other
    // request on the same worker thread for the duration.
    if let Err(e) = tokio::fs::copy(&tmp_path, &orig_path).await {
        tracing::warn!(path = %orig_path.display(), error = %e, "could not save original file");
    }

    tracing::info!(
        document_id = %document_id,
        filename = %filename,
        chunks = chunks.len(),
        pages = ?page_count,
        "document ingested"
    );

    Json(json!({
        "id":          document_id,
        "filename":    filename,
        "page_count":  page_count,
        "doc_type":    ext,
        "chunk_count": chunks.len(),
        "upload_date": upload_date,
    }))
    .into_response()
}

// ── GET /api/documents/{id}/download ─────────────────────────────────────────

pub async fn download(
    State(state): State<AppState>,
    _claims: Claims,
    Path(document_id): Path<String>,
) -> Response {
    let doc = match db::documents::find_by_id(&state.db, &document_id).await {
        Ok(Some(d)) if d.is_deleted == 0 => d,
        Ok(_) => return err(StatusCode::NOT_FOUND, "document not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    let path = state.storage.path_for(&document_id, &doc.filename);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!(
                        "attachment; filename=\"{}\"",
                        sanitize_header_filename(&doc.filename)
                    ),
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => err(StatusCode::NOT_FOUND, "file not found on disk"),
    }
}

// ── DELETE /api/documents/{id} ────────────────────────────────────────────────

pub async fn delete(
    State(state): State<AppState>,
    claims: Claims,
    Path(document_id): Path<String>,
) -> Response {
    // RBAC: delete requires admin|super_user.
    if !claims.role.can_delete() {
        return err(StatusCode::FORBIDDEN, "insufficient permissions to delete documents");
    }
    match purge_document(&state, &document_id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "document not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// The SINGLE entry point for deleting a document, which is what keeps the
/// SQLite↔Qdrant sync invariant enforceable in one place.
///
/// Mandatory order: **Qdrant FIRST, then SQLite**, then the original file.
/// The Qdrant deletion is idempotent and safe even on zero vectors, so the
/// admin endpoint can also use it to clean up orphans.
///
/// Returns `Ok(false)` when the document does not exist or was already
/// soft-deleted.
pub(crate) async fn purge_document(state: &AppState, document_id: &str) -> anyhow::Result<bool> {
    // Fetch the filename, needed for the file cleanup, before touching the stores.
    let doc = db::documents::find_by_id(&state.db, document_id).await?;

    // INVARIANT: Qdrant FIRST. If it fails we stop, leaving SQLite consistent
    // — the document is still "active" — rather than an orphan with no
    // vectors.
    state.qdrant.delete_document(document_id).await?;

    // SQLite second. soft_delete only touches rows with is_deleted = 0, so it
    // returns false when the document did not exist or was already deleted.
    let removed = db::documents::soft_delete(&state.db, document_id).await?;

    // Best-effort: remove the original file and its {base}/{id} directory.
    if removed {
        if let Some(d) = &doc {
            let file_path = state.storage.path_for(document_id, &d.filename);
            let _ = tokio::fs::remove_file(&file_path).await;
            // parent() is {base}/{document_id}; remove_dir fails, and is
            // ignored, when it is not empty, so it never touches anything that
            // is not ours.
            if let Some(parent) = file_path.parent() {
                let _ = tokio::fs::remove_dir(parent).await;
            }
        }
    }

    Ok(removed)
}

/// Where an upload is staged while it is parsed, and with what permissions.
///
/// Under the data directory, not `std::env::temp_dir()`. Two reasons, and
/// the first is the one that matters: on most Linux installs `/tmp` is a
/// tmpfs, which is RAM — so streaming a large upload there put it straight
/// back into the memory that streaming to disk exists to avoid, and on a
/// small ARM board could fill it. The second is that the data directory is
/// on the same filesystem as the originals store, which keeps the copy at
/// step 7 local instead of crossing devices.
///
/// Created with mode 0600 on Unix. The default is 0644: on a shared host,
/// every other local user could read whatever was being ingested for as
/// long as the parse took.
async fn upload_tmp_path(state: &AppState, ext: &str) -> anyhow::Result<std::path::PathBuf> {
    let dir = state.settings.data.data_path().join("tmp");
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating upload staging dir {}", dir.display()))?;
    Ok(dir.join(format!("{}.{ext}", uuid::Uuid::new_v4())))
}

/// Creates the staging file with owner-only permissions where the platform
/// has them. Split out so `stream_upload_to_file` stays about streaming.
async fn create_private_file(path: &std::path::Path) -> std::io::Result<tokio::fs::File> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    // tokio's own OpenOptions::mode, not the std extension trait — it is
    // already cfg(unix) on this side.
    #[cfg(unix)]
    opts.mode(0o600);
    opts.open(path).await
}

/// Owns the upload temp file from reception: dropping it removes the file,
/// so every early return after step 1 — parse failure, empty chunks, failed
/// upsert — cleans up without a `remove_file` on each path.
struct RemoveOnDrop(std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        // The one std::fs call left on this path, and it has to be: Drop is
        // not async, so there is nowhere to await tokio::fs here. An unlink
        // is a metadata operation rather than a transfer, so the executor
        // stall is a syscall rather than the length of a file — which is
        // what E4 was actually about.
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Why the streamed receive failed. The partial temp file is already
/// removed in every case, so the caller only picks a status code.
#[derive(Debug)]
enum ReceiveError {
    /// Body exceeded `max_bytes` — 413.
    TooLarge,
    /// A body chunk could not be read (client/network fault) — 400.
    Read(anyhow::Error),
    /// The temp file could not be created or written — 500.
    Write(anyhow::Error),
}

/// Streams one multipart body to `tmp_path`, hashing incrementally and
/// rejecting as soon as `max_bytes` is exceeded — the body is never held
/// in RAM (issue #43). Returns the total bytes and the hex sha256: the
/// digest is identical to a one-shot hash of the same bytes, so the
/// provenance chain downstream is unaffected. A partial file is removed on
/// every error path, including the limit breach (note the explicit `drop`
/// before removing: on Windows an open file cannot be removed).
async fn stream_upload_to_file<S, C, E>(
    chunks: std::pin::Pin<&mut S>,
    tmp_path: &std::path::Path,
    max_bytes: u64,
) -> Result<(u64, String), ReceiveError>
where
    S: futures_util::Stream<Item = Result<C, E>>,
    C: AsRef<[u8]>,
    E: std::fmt::Display,
{
    let mut out = create_private_file(tmp_path)
        .await
        .map_err(|e| ReceiveError::Write(anyhow::anyhow!("create temp upload file: {e}")))?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut chunks = chunks;
    while let Some(item) = chunks.next().await {
        let chunk = match item {
            Ok(c) => c,
            Err(e) => {
                drop(out);
                let _ = tokio::fs::remove_file(tmp_path).await;
                return Err(ReceiveError::Read(anyhow::anyhow!("read upload body: {e}")));
            }
        };
        let bytes = chunk.as_ref();
        total += bytes.len() as u64;
        if total > max_bytes {
            drop(out);
            let _ = tokio::fs::remove_file(tmp_path).await;
            return Err(ReceiveError::TooLarge);
        }
        if let Err(e) = out.write_all(bytes).await {
            drop(out);
            let _ = tokio::fs::remove_file(tmp_path).await;
            return Err(ReceiveError::Write(anyhow::anyhow!(
                "write temp upload file: {e}"
            )));
        }
        hasher.update(bytes);
    }
    // `tokio::fs::File` dispatches writes to a blocking pool and `drop` does
    // not wait for them: without this flush the function could return while
    // the last chunks are still in flight, and the parser would extract text
    // from a truncated file. Flush errors take the same path as write
    // errors. (Flush gets the bytes to the OS, which is all the parser —
    // reading back through the page cache — needs.)
    if let Err(e) = out.flush().await {
        drop(out);
        let _ = tokio::fs::remove_file(tmp_path).await;
        return Err(ReceiveError::Write(anyhow::anyhow!(
            "flush temp upload file: {e}"
        )));
    }
    drop(out);
    Ok((total, format!("{:x}", hasher.finalize())))
}

/// Makes a stored filename safe to interpolate into a quoted
/// `Content-Disposition` header value.
///
/// The filename is attacker-controlled (multipart `file_name()`, stored
/// verbatim) while `storage::path_for` only neutralises *path* traversal —
/// nothing stops `"`, `\` or CR/LF from reaching the header, where a quote
/// or backslash breaks out of the quoted-string and CR/LF splits the
/// response (axum 500s the download at best). Mapping the three header
/// metacharacters plus ASCII controls to `_` keeps legitimate names —
/// including non-ASCII ones like `Relazione 2026.pdf` — byte-identical.
fn sanitize_header_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{create_private_file, sanitize_header_filename, stream_upload_to_file, ReceiveError};
    use futures_util::stream;
    use sha2::{Digest, Sha256};

    fn tmp_in(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
        dir.path().join(name)
    }

    /// Incrementality must be invisible: same length, same digest, same
    /// bytes on disk as buffering the whole body first.
    /// The staging file must not be world-readable: on a shared host the
    /// default 0644 let every other local user read whatever was being
    /// ingested, for as long as the parse took.
    #[cfg(unix)]
    #[tokio::test]
    async fn staging_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tmp = tmp_in(&dir, "up.bin");
        let f = create_private_file(&tmp).await.unwrap();
        drop(f);
        let mode = std::fs::metadata(&tmp).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
    }

    /// create_new, so a staging path that somehow already exists is an
    /// error rather than a file we truncate — the uuid makes a collision
    /// implausible, not impossible.
    #[cfg(unix)]
    #[tokio::test]
    async fn staging_file_refuses_to_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = tmp_in(&dir, "taken.bin");
        std::fs::write(&tmp, b"existing").unwrap();
        assert!(create_private_file(&tmp).await.is_err());
        assert_eq!(std::fs::read(&tmp).unwrap(), b"existing");
    }

    #[tokio::test]
    async fn streams_chunks_with_one_shot_digest() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = tmp_in(&dir, "up.bin");
        let chunks = stream::iter(vec![
            Ok::<Vec<u8>, std::io::Error>(b"hello ".to_vec()),
            Ok(b"world".to_vec()),
        ]);
        futures_util::pin_mut!(chunks);
        let (len, sha) = stream_upload_to_file(chunks, &tmp, 1024).await.unwrap();
        assert_eq!(len, 11);
        let mut expected = Sha256::new();
        expected.update(b"hello world");
        assert_eq!(sha, format!("{:x}", expected.finalize()));
        assert_eq!(std::fs::read(&tmp).unwrap(), b"hello world");
    }

    #[tokio::test]
    async fn over_limit_rejects_and_removes_partial() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = tmp_in(&dir, "up.bin");
        let chunks = stream::iter(vec![
            Ok::<Vec<u8>, std::io::Error>(b"12345678".to_vec()),
            Ok(b"way-too-much".to_vec()),
        ]);
        futures_util::pin_mut!(chunks);
        let err = stream_upload_to_file(chunks, &tmp, 10).await.unwrap_err();
        assert!(matches!(err, ReceiveError::TooLarge));
        assert!(!tmp.exists(), "partial file must not survive rejection");
    }

    #[tokio::test]
    async fn body_read_error_maps_to_read_and_removes_partial() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = tmp_in(&dir, "up.bin");
        let chunks = stream::iter(vec![
            Ok::<Vec<u8>, std::io::Error>(b"partial".to_vec()),
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "boom")),
        ]);
        futures_util::pin_mut!(chunks);
        let err = stream_upload_to_file(chunks, &tmp, 1024).await.unwrap_err();
        assert!(matches!(err, ReceiveError::Read(_)));
        assert!(!tmp.exists(), "partial file must not survive a broken body");
    }

    #[tokio::test]
    async fn unwritable_destination_maps_to_write() {
        // `blocker` is a regular file, so nothing can be created under it —
        // on any platform, without touching real system paths.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let tmp = blocker.join("up.bin");
        let chunks = stream::iter(vec![Ok::<Vec<u8>, std::io::Error>(b"x".to_vec())]);
        futures_util::pin_mut!(chunks);
        let err = stream_upload_to_file(chunks, &tmp, 1024).await.unwrap_err();
        assert!(matches!(err, ReceiveError::Write(_)));
    }

    #[tokio::test]
    async fn empty_body_reports_zero_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = tmp_in(&dir, "up.bin");
        let chunks = stream::iter(Vec::<Result<Vec<u8>, std::io::Error>>::new());
        futures_util::pin_mut!(chunks);
        let (len, sha) = stream_upload_to_file(chunks, &tmp, 1024).await.unwrap();
        assert_eq!(len, 0);
        let mut expected = Sha256::new();
        expected.update(b"");
        assert_eq!(sha, format!("{:x}", expected.finalize()));
    }

    #[test]
    fn plain_and_unicode_names_pass_through() {
        assert_eq!(sanitize_header_filename("report.pdf"), "report.pdf");
        assert_eq!(
            sanitize_header_filename("Relazione 2026.pdf"),
            "Relazione 2026.pdf"
        );
    }

    #[test]
    fn quotes_backslashes_and_crlf_become_underscores() {
        assert_eq!(sanitize_header_filename("evil\".pdf"), "evil_.pdf");
        assert_eq!(sanitize_header_filename("a\\b.pdf"), "a_b.pdf");
        assert_eq!(
            sanitize_header_filename("a\r\nX-Evil-1.pdf"),
            "a__X-Evil-1.pdf"
        );
    }

    #[test]
    fn result_is_header_safe() {
        for hostile in [
            "x\".pdf",
            "x\\.pdf",
            "x\r.pdf",
            "x\n.pdf",
            "x\u{0}.pdf",
            "x\u{7f}.pdf",
        ] {
            let clean = sanitize_header_filename(hostile);
            assert!(
                !clean.contains(['"', '\\', '\r', '\n']) && !clean.chars().any(|c| c.is_control()),
                "unsafe output {clean:?} for input {hostile:?}"
            );
        }
    }
}
