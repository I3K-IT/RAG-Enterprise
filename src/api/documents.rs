//! Document management endpoints (MAPPA §3):
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
    // RBAC (parità Python: require_upload_permission → admin|super_user).
    if !claims.role.can_upload() {
        return err(StatusCode::FORBIDDEN, "permessi insufficienti per caricare documenti");
    }
    // La guardia resta viva (quindi active_ingestions > 0, vedi
    // AppState::ingestion_blocks_queries) per l'INTERA finestra
    // unload → estrazione/chunk/embed → reload, non solo la fase pesante:
    // se calasse prima del reload, una query concorrente rifarebbe caricare
    // eullm da sola mentre l'embedding sta ancora usando la VRAM liberata.
    let _ingestion_guard = crate::state::IngestionGuard::start(&state.active_ingestions);
    let unload_enabled = state.settings.eullm.unload_during_ingestion;
    let swap_enabled = state.settings.embeddings.swap_during_ingestion;

    // Ordine: libera VRAM (eullm) PRIMA di occuparla (bge-m3) — e al
    // termine, il rientro è speculare: bge-m3 lascia la VRAM PRIMA che
    // eullm la riprenda, altrimenti il reload di eullm (--fit legge la
    // VRAM libera in quel momento) la troverebbe ancora occupata.
    if unload_enabled {
        if let Err(e) = state.eullm.unload().await {
            tracing::error!(error = %e, "eullm: unload pre-ingestione fallito, procedo comunque (nessuna VRAM liberata)");
        }
    }
    if swap_enabled {
        if let Err(e) = swap_embeddings_blocking(&state, true).await {
            tracing::error!(error = %e, "embedding: swap su GPU fallito, l'ingestione userà la CPU (più lenta)");
        }
    }

    let response = process_upload(&state, multipart).await;

    if swap_enabled {
        if let Err(e) = swap_embeddings_blocking(&state, false).await {
            tracing::error!(error = %e, "embedding: swap su CPU fallito — bge-m3 potrebbe essere ancora in VRAM, verifica manualmente");
        }
    }
    if unload_enabled {
        if let Err(e) = state.eullm.reload().await {
            tracing::error!(error = %e, "eullm: reload post-ingestione fallito — il modello potrebbe non essere in VRAM, verifica manualmente");
        }
    }

    response
}

/// Esegue lo swap del device dell'embedding (bloccante: mmap + copia pesi)
/// fuori dall'executor async. `to_gpu=true` verso GPU (inizio ingestione),
/// `false` verso CPU (fine ingestione) — vedi
/// EmbeddingsSettings::swap_during_ingestion.
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
    // Timer coperto anche se --bench-live è spento: un Instant::now() costa
    // nulla di rilevante, non vale la pena condizionarlo.
    let extract_start = std::time::Instant::now();
    let (text, page_count) = match tokio::task::spawn_blocking({
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

    // 5. Embed — CPU/GPU bound, spawn_blocking
    let embed_start = std::time::Instant::now();
    let embeddings_svc = state.embeddings.clone();
    let chunk_strs: Vec<String> = chunks.clone();
    let embeddings = match tokio::task::spawn_blocking(move || {
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
    };
    let embed_time = embed_start.elapsed();

    // 6. Build Qdrant payloads and upsert
    let document_id = uuid::Uuid::new_v4().to_string();
    let upload_date = Utc::now().to_rfc3339();

    let payloads: Vec<ChunkPayload> = chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| ChunkPayload {
            document_id: document_id.clone(),
            chunk_index: i,
            filename: filename.clone(),
            upload_date: upload_date.clone(),
            text: chunk.clone(),
            chunk_size: chunk.len(),
            document_type: ext.clone(),
            structured_fields: None,
        })
        .collect();

    let upsert_start = std::time::Instant::now();
    if let Err(e) = state.qdrant.upsert(&embeddings, &payloads).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("qdrant upsert: {e}"));
    }
    let upsert_time = upsert_start.elapsed();

    // --bench-live: registra questa ingestione reale (vedi bench::LiveRecorder).
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
    // RBAC (parità Python: require_delete_permission → admin|super_user).
    if !claims.role.can_delete() {
        return err(StatusCode::FORBIDDEN, "permessi insufficienti per eliminare documenti");
    }
    match purge_document(&state, &document_id).await {
        Ok(true) => Json(json!({ "deleted": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "document not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// Punto di ingresso UNICO per la cancellazione di un documento (vedi
/// invariante sync SQLite↔Qdrant — "un solo punto di ingresso per delete").
/// Ordine obbligatorio: **Qdrant PRIMA, poi SQLite**, poi il file originale.
/// La cancellazione Qdrant è idempotente (safe anche su 0 vettori), così
/// l'endpoint admin può usarla anche per ripulire eventuali orfani.
/// Ritorna `Ok(false)` se il documento non esiste o era già soft-deleted.
pub(crate) async fn purge_document(state: &AppState, document_id: &str) -> anyhow::Result<bool> {
    // Recupera il filename (per il cleanup del file) prima di toccare gli store.
    let doc = db::documents::find_by_id(&state.db, document_id).await?;

    // INVARIANTE: Qdrant PRIMA. Se fallisce, ci fermiamo: SQLite resta coerente
    // (documento ancora "attivo") anziché diventare un orfano senza vettori.
    state.qdrant.delete_document(document_id).await?;

    // SQLite dopo. soft_delete tocca solo righe con is_deleted = 0: ritorna
    // false se il documento non esisteva o era già cancellato.
    let removed = db::documents::soft_delete(&state.db, document_id).await?;

    // Best-effort: rimuove il file originale e la sua cartella {base}/{id}.
    if removed {
        if let Some(d) = &doc {
            let file_path = state.storage.path_for(document_id, &d.filename);
            let _ = std::fs::remove_file(&file_path);
            // parent() = {base}/{document_id}; remove_dir fallisce (ignorato) se
            // non è vuota, quindi non tocca mai nulla che non sia nostro.
            if let Some(parent) = file_path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }

    Ok(removed)
}
