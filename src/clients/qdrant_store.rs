//! Qdrant connector.
//!
//! Parità con Python (MAPPA §7 — qdrant_connector.py):
//! - Collection: "rag_documents", size 1024, distance COSINE
//! - insert_vectors: batch 1000, wait=true, id=UUID4 str
//! - Payload fields: document_id, chunk_index, filename, upload_date, text, chunk_size, document_type
//! - search: score_threshold opzionale; restituisce {id, similarity, metadata}
//! - delete_document: filtra per document_id
//!
//! INVARIANTE (CLAUDE.md): delete_document / reindex devono toccare SQLite E Qdrant.
//! Un solo punto di ingresso per delete/reindex — mai aggirarlo.

use anyhow::{Context, Result};
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, Condition,
        PointStruct, SearchPointsBuilder, UpsertPointsBuilder,
        VectorParamsBuilder, VectorsConfig, vectors_config::Config,
    },
};
use serde::{Deserialize, Serialize};

pub const COLLECTION: &str = "rag_documents";
pub const VECTOR_DIM: u64 = 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkPayload {
    pub document_id: String,
    pub chunk_index: usize,
    pub filename: String,
    pub upload_date: String,
    pub text: String,
    pub chunk_size: usize,
    pub document_type: String,
}

#[derive(Debug)]
pub struct SearchHit {
    pub id: String,
    pub similarity: f32,
    pub payload: ChunkPayload,
}

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
            self.client.create_collection(
                CreateCollectionBuilder::new(&self.collection)
                    .vectors_config(VectorsConfig {
                        config: Some(Config::Params(
                            VectorParamsBuilder::new(VECTOR_DIM, Distance::Cosine).build(),
                        )),
                    })
            ).await?;
            tracing::info!(collection = %self.collection, "created Qdrant collection");
        }
        Ok(())
    }

    /// Upsert chunks in batches of 1000 (parità con Python: BATCH_SIZE=1000, wait=true, id=UUID4).
    pub async fn upsert(&self, embeddings: &[Vec<f32>], payloads: &[ChunkPayload]) -> Result<()> {
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
                    let payload = Payload::try_from(serde_json::json!({
                        "document_id":   p.document_id,
                        "chunk_index":   p.chunk_index as i64,
                        "filename":      p.filename,
                        "upload_date":   p.upload_date,
                        "text":          p.text,
                        "chunk_size":    p.chunk_size as i64,
                        "document_type": p.document_type,
                    }))
                    .expect("static JSON shape is always valid");
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

    /// Vector similarity search.
    pub async fn search(
        &self,
        query_vec: Vec<f32>,
        top_k: u64,
        score_threshold: Option<f32>,
    ) -> Result<Vec<SearchHit>> {
        let mut builder =
            SearchPointsBuilder::new(&self.collection, query_vec, top_k).with_payload(true);
        if let Some(threshold) = score_threshold {
            builder = builder.score_threshold(threshold);
        }
        let resp = self.client.search_points(builder).await?;
        let hits = resp
            .result
            .into_iter()
            .filter_map(|hit| {
                let raw: serde_json::Map<String, serde_json::Value> = hit
                    .payload
                    .into_iter()
                    .map(|(k, v)| (k, v.into()))
                    .collect();
                let payload: ChunkPayload =
                    serde_json::from_value(serde_json::Value::Object(raw)).ok()?;
                // Extract point id as string (format depends on id type: uint / uuid)
                let id = hit.id.and_then(|pid| pid.point_id_options).map(|o| format!("{o:?}"))?;
                Some(SearchHit { id, similarity: hit.score, payload })
            })
            .collect();
        Ok(hits)
    }

    /// Delete all chunks for a document_id.
    /// INVARIANTE: chiamare PRIMA di aggiornare SQLite.
    pub async fn delete_document(&self, document_id: &str) -> Result<()> {
        let filter = Filter::must([Condition::matches("document_id", document_id.to_owned())]);
        self.client
            .delete_points(DeletePointsBuilder::new(&self.collection).points(filter))
            .await?;
        Ok(())
    }
}
