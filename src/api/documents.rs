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
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::auth::jwt::Claims;
use crate::bench;
use crate::db;
use crate::rag::vector_store::ChunkPayload;
use crate::documents::parser;
use crate::rag::chunker;
use crate::state::AppState;

fn err(status: StatusCode, msg: impl std::fmt::Display) -> Response {
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
    // The guard stays alive — keeping active_ingestions > 0, see
    // AppState::ingestion_blocks_queries — for the WHOLE window of
    // unload → extract/chunk/embed → reload, not just the heavy part. If it
    // were dropped before the reload, a concurrent query would load eullm
    // again on its own while the embedding model is still using the freed
    // VRAM.
    let _ingestion_guard = crate::state::IngestionGuard::start(&state.active_ingestions);
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
    // 1. Read file bytes from multipart field "file"
    let mut filename = String::new();
    let mut file_bytes: Vec<u8> = Vec::new();

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name().unwrap_or("") == "file" {
                    filename = field.file_name().unwrap_or("upload").to_string();
                    match field.bytes().await {
                        Ok(b) => {
                            file_bytes = b.to_vec();
                            break;
                        }
                        Err(e) => return err(StatusCode::BAD_REQUEST, e),
                    }
                }
            }
            Ok(None) => break,
            Err(e) => return err(StatusCode::BAD_REQUEST, e),
        }
    }

    if file_bytes.is_empty() {
        return err(StatusCode::BAD_REQUEST, "no file in multipart request (field name: \"file\")");
    }

    // Content identity for provenance_id (see rag::chunker::provenance_id):
    // anchored to the uploaded bytes, NOT to document_id below (a fresh UUID
    // every upload) — re-ingesting this same file must yield the same hash,
    // and therefore the same provenance_id per chunk, given an unchanged
    // chunking configuration.
    let content_sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&file_bytes);
        format!("{:x}", hasher.finalize())
    };

    // 2. Write to a named temp file preserving the extension (parser dispatches by ext)
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let tmp_path = std::env::temp_dir().join(format!("{}.{ext}", uuid::Uuid::new_v4()));

    if let Err(e) = std::fs::write(&tmp_path, &file_bytes) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("write temp: {e}"));
    }

    // 3. Extract text (sync, possibly heavy — run off the async executor).
    // Timed even when --bench-live is off: an Instant::now() costs nothing
    // worth conditionalising.
    let extract_start = std::time::Instant::now();
    let extracted = match tokio::task::spawn_blocking({
        let tmp = tmp_path.clone();
        let data_dir = state.settings.data.data_path();
        move || {
            let r = parser::extract_text(&tmp, &data_dir);
            let _ = std::fs::remove_file(&tmp);
            r
        }
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("parse task: {e}")),
    };
    let extract_time = extract_start.elapsed();
    let parser::ExtractedText { text, page_count, pages } = extracted;

    // 4. Chunk
    let chunk_start = std::time::Instant::now();
    let chunks = chunker::split_text(&text);
    let chunk_time = chunk_start.elapsed();
    if chunks.is_empty() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no text could be extracted from this document",
        );
    }

    // 4b. Prepend the nearest preceding structural heading ("Article 99",
    // "Chapter XII", ...) to any chunk that doesn't already start with it —
    // see chunker::inject_heading_context. This is what actually gets
    // embedded and stored; `chunks[i].start_byte`/`end_byte` (used below for
    // page lookups and citation spans) still point at the real source
    // location, untouched by the injected label.
    let chunk_texts = chunker::inject_heading_context(&text, &chunks);

    // 5. Embed. Candle is CPU/GPU-bound (spawn_blocking); eullm is an HTTP
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

    // 6. Build Qdrant payloads and upsert
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
                text: chunk_texts[i].clone(),
                chunk_size: chunk_texts[i].len(),
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

    // 7. Persist metadata in SQLite
    if let Err(e) =
        db::documents::insert(&state.db, &document_id, &filename, page_count, &ext, chunks.len())
            .await
    {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("db insert: {e}"));
    }

    // 8. Save original file for later download (best-effort)
    let orig_path = state.storage.path_for(&document_id, &filename);
    if let Some(parent) = orig_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&orig_path, &file_bytes) {
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
                    format!("attachment; filename=\"{}\"", doc.filename),
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
            let _ = std::fs::remove_file(&file_path);
            // parent() is {base}/{document_id}; remove_dir fails, and is
            // ignored, when it is not empty, so it never touches anything that
            // is not ours.
            if let Some(parent) = file_path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }

    Ok(removed)
}
