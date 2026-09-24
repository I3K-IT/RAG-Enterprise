//! Qdrant connector — implements VectorStore.
//!
//! Parity with the Python qdrant_connector.py:
//! - Collection: "rag_documents", size 1024, distance COSINE
//! - upsert: batch 1000, wait=true, id=UUID4 str
//! - Payload: document_id, chunk_index, filename, upload_date, text, chunk_size,
//!   document_type, structured_fields (optional), plus the Source Provenance
//!   Foundation fields (all optional — absent on points written before they
//!   existed): source_start_byte, source_end_byte, page_start, page_end,
//!   provenance_id, retrieval_text — see ChunkPayload.
//! - search: optional score_threshold; returns {id, similarity, payload}
//! - delete_document: filters by document_id

use anyhow::{Context, Result};
use async_trait::async_trait;
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder,
        DeletePointsBuilder, Distance, FieldType, Filter, PointStruct,
        SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
        VectorsConfig, vectors_config::Config,
    },
};

use crate::rag::vector_store::{ChunkPayload, SearchHit, VectorStore};

pub const VECTOR_DIM: u64 = 1024;

/// The one payload field this code filters on — see ensure_document_id_index.
const DOCUMENT_ID_FIELD: &str = "document_id";

pub struct QdrantStore {
    client: Qdrant,
    collection: String,
}

impl QdrantStore {
    pub async fn new(url: &str, collection: &str) -> Result<Self> {
        let client = Qdrant::from_url(url).build()?;
        let store = Self { client, collection: collection.to_owned() };
        store.ensure_collection().await?;
        Ok(store)
    }

    async fn ensure_collection(&self) -> Result<()> {
        let exists = self.client.collection_exists(&self.collection).await?;
        if !exists {
            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&self.collection).vectors_config(VectorsConfig {
                        config: Some(Config::Params(
                            VectorParamsBuilder::new(VECTOR_DIM, Distance::Cosine).build(),
                        )),
                    }),
                )
                .await?;
            tracing::info!(collection = %self.collection, "Qdrant collection created");
        }
        self.ensure_document_id_index().await;
        Ok(())
    }

    /// Payload index on `document_id`.
    ///
    /// Without one, Qdrant answers a `document_id` filter — which is every
    /// delete_document call — by reading the payload of every point in the
    /// collection. At ten thousand documents, removing one of them means
    /// scanning millions of points to find its few hundred.
    ///
    /// Deliberately outside the `if !exists` above: a collection created by
    /// an earlier version is already there and has no index, and would never
    /// acquire one if this only ran at creation time. Qdrant treats a repeat
    /// request for an index that already exists as a no-op, so running it on
    /// every startup is free.
    ///
    /// Returns nothing, and logs rather than propagates: an index is a matter
    /// of how fast a delete is, not whether the engine works, so a Qdrant
    /// that refuses it must not be a Qdrant this binary refuses to start
    /// against.
    async fn ensure_document_id_index(&self) {
        match self
            .client
            .create_field_index(CreateFieldIndexCollectionBuilder::new(
                &self.collection,
                DOCUMENT_ID_FIELD,
                FieldType::Keyword,
            ))
            .await
        {
            Ok(_) => tracing::debug!(
                collection = %self.collection,
                field = DOCUMENT_ID_FIELD,
                "Qdrant payload index in place"
            ),
            Err(e) => tracing::warn!(
                collection = %self.collection,
                field = DOCUMENT_ID_FIELD,
                error = %e,
                "Qdrant payload index not created: per-document deletes will scan \
                 the whole collection"
            ),
        }
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    /// Upsert in batches of 1000 (BATCH_SIZE=1000, wait=true, id=UUID4).
    async fn upsert(&self, embeddings: &[Vec<f32>], payloads: &[ChunkPayload]) -> Result<()> {
        if embeddings.len() != payloads.len() {
            anyhow::bail!(
                "embeddings/payloads length mismatch: {} vs {}",
                embeddings.len(),
                payloads.len()
            );
        }

        const BATCH: usize = 1000;
        let mut i = 0;
        while i < embeddings.len() {
            let end = (i + BATCH).min(embeddings.len());
            let points: Vec<PointStruct> = embeddings[i..end]
                .iter()
                .zip(payloads[i..end].iter())
                .map(|(emb, p)| {
                    let id = uuid::Uuid::new_v4().to_string();
                    let mut obj = serde_json::json!({
                        "document_id":   p.document_id,
                        "chunk_index":   p.chunk_index as i64,
                        "filename":      p.filename,
                        "upload_date":   p.upload_date,
                        "text":          p.text,
                        "chunk_size":    p.chunk_size as i64,
                        "document_type": p.document_type,
                    });
                    if let Some(sf) = &p.structured_fields {
                        obj["structured_fields"] = sf.clone();
                    }
                    if let Some(v) = p.source_start_byte {
                        obj["source_start_byte"] = serde_json::json!(v as i64);
                    }
                    if let Some(v) = p.source_end_byte {
                        obj["source_end_byte"] = serde_json::json!(v as i64);
                    }
                    if let Some(v) = p.page_start {
                        obj["page_start"] = serde_json::json!(v);
                    }
                    if let Some(v) = p.page_end {
                        obj["page_end"] = serde_json::json!(v);
                    }
                    if let Some(v) = &p.provenance_id {
                        obj["provenance_id"] = serde_json::json!(v);
                    }
                    if let Some(v) = &p.retrieval_text {
                        obj["retrieval_text"] = serde_json::json!(v);
                    }
                    let payload = Payload::try_from(obj).expect("valid JSON shape");
                    PointStruct::new(id, emb.clone(), payload)
                })
                .collect();

            self.client
                .upsert_points(UpsertPointsBuilder::new(&self.collection, points).wait(true))
                .await
                .with_context(|| format!("upsert batch {i}..{end}"))?;

            i = end;
        }
        Ok(())
    }

    async fn search(
        &self,
        query_vec: Vec<f32>,
        top_k: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<SearchHit>> {
        let mut builder =
            SearchPointsBuilder::new(&self.collection, query_vec, top_k).with_payload(true);
        if let Some(t) = score_threshold {
            builder = builder.score_threshold(t);
        }
        let resp = self.client.search_points(builder).await?;
        let returned = resp.result.len();
        let hits: Vec<SearchHit> = resp
            .result
            .into_iter()
            .filter_map(|hit| {
                let id = hit.id.as_ref().map(|i| format!("{i:?}")).unwrap_or_default();
                let raw: serde_json::Map<String, serde_json::Value> =
                    hit.payload.into_iter().map(|(k, v)| (k, v.into())).collect();
                match serde_json::from_value::<ChunkPayload>(serde_json::Value::Object(raw)) {
                    Ok(payload) => Some(SearchHit { similarity: hit.score, payload }),
                    // A point Qdrant matched but whose payload will not parse
                    // is a chunk the user's question found and the answer will
                    // not contain — written by an older schema, or by another
                    // tool against the same collection. Dropping it silently
                    // is what made "the answer ignores a document I know is in
                    // there" impossible to explain from the outside.
                    Err(e) => {
                        tracing::warn!(
                            point_id = %id,
                            error = %e,
                            "Qdrant: payload will not deserialize, point excluded from the results"
                        );
                        None
                    }
                }
            })
            .collect();
        if hits.len() != returned {
            tracing::warn!(
                returned,
                usable = hits.len(),
                "Qdrant: dropped {} of {returned} points with unreadable payloads",
                returned - hits.len()
            );
        }
        Ok(hits)
    }

    async fn delete_document(&self, document_id: &str) -> Result<()> {
        let filter =
            Filter::must([Condition::matches(DOCUMENT_ID_FIELD, document_id.to_owned())]);
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(filter)
                    .wait(true),
            )
            .await
            .with_context(|| format!("qdrant delete_document {document_id}"))?;
        tracing::info!(document_id = %document_id, "Qdrant vectors deleted");
        Ok(())
    }
}
