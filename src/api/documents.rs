//! Document management endpoints (MAPPA §3):
//! GET    /api/documents
//! POST   /api/documents/upload   — multipart/form-data, field "file"
//! DELETE /api/documents/{id}

use axum::{
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::json;

use crate::auth::jwt::Claims;
use crate::clients::qdrant_store::ChunkPayload;
use crate::db;
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
    _claims: Claims,
    mut multipart: Multipart,
) -> Response {
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

    // 3. Extract text (sync, possibly heavy — run off the async executor)
    let (text, page_count) = match tokio::task::spawn_blocking({
        let tmp = tmp_path.clone();
        move || {
            let r = parser::extract_text(&tmp);
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

    // 4. Chunk
    let chunks = chunker::split_text(&text);
    if chunks.is_empty() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no text could be extracted from this document",
        );
    }

    // 5. Embed — CPU/GPU bound, spawn_blocking
    let embeddings_svc = state.embeddings.clone();
    let chunk_strs: Vec<String> = chunks.clone();
    let embeddings = match tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = chunk_strs.iter().map(|s| s.as_str()).collect();
        embeddings_svc.embed_texts(&refs)
    })
    .await
    {
        Ok(Ok(e)) => e,
        Ok(Err(e)) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("embedding: {e}")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("embed task: {e}")),
    };

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
        })
        .collect();

    if let Err(e) = state.qdrant.upsert(&embeddings, &payloads).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("qdrant upsert: {e}"));
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
    _claims: Claims,
    Path(document_id): Path<String>,
) -> Response {
    // Verify the document exists and is not already soft-deleted
    match db::documents::find_by_id(&state.db, &document_id).await {
        Ok(Some(doc)) if doc.is_deleted != 0 => {
            return err(StatusCode::NOT_FOUND, "document not found")
        }
        Ok(None) => return err(StatusCode::NOT_FOUND, "document not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Ok(Some(_)) => {}
    }

    // INVARIANTE (CLAUDE.md): Qdrant first, then SQLite — never reverse this order.
    if let Err(e) = state.qdrant.delete_document(&document_id).await {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("qdrant delete: {e}"),
        );
    }

    let doc = db::documents::find_by_id(&state.db, &document_id)
        .await
        .ok()
        .flatten();

    match db::documents::soft_delete(&state.db, &document_id).await {
        Ok(true) => {
            // Best-effort: remove stored original file
            if let Some(d) = doc {
                let file_path = state.storage.path_for(&document_id, &d.filename);
                let dir_path = state.storage.path_for(&document_id, "");
                let _ = std::fs::remove_file(&file_path);
                let _ = std::fs::remove_dir(dir_path.parent().unwrap_or(&file_path));
            }
            Json(json!({ "deleted": true })).into_response()
        }
        Ok(false) => err(StatusCode::NOT_FOUND, "document not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
