//! Qdrant connector — implementa VectorStore.
//!
//! Parity with the Python qdrant_connector.py:
//! - Collection: "rag_documents", size 1024, distance COSINE
//! - upsert: batch 1000, wait=true, id=UUID4 str
//! - Payload: document_id, chunk_index, filename, upload_date, text, chunk_size,
//!            document_type, structured_fields (opzionale)
//! - search: score_threshold opzionale; restituisce {id, similarity, payload}
//! - delete_document: filters by document_id

use anyhow::{Context, Result};
use async_trait::async_trait;
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter,
        PointStruct, SearchPointsBuilder, UpsertPointsBuilder,
        VectorParamsBuilder, VectorsConfig, vectors_config::Config,
    },
};

use crate::rag::vector_store::{ChunkPayload, SearchHit, VectorStore};

pub const VECTOR_DIM: u64 = 1024;

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
            tracing::info!(collection = %self.collection, "collection Qdrant creata");
        }
        Ok(())
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    /// Upsert in batch da 1000 (parità Python: BATCH_SIZE=1000, wait=true, id=UUID4).
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
                    let payload = Payload::try_from(obj).expect("shape JSON valido");
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
        let hits = resp
            .result
            .into_iter()
            .filter_map(|hit| {
                let raw: serde_json::Map<String, serde_json::Value> =
                    hit.payload.into_iter().map(|(k, v)| (k, v.into())).collect();
                let payload: ChunkPayload =
                    serde_json::from_value(serde_json::Value::Object(raw)).ok()?;
                Some(SearchHit { similarity: hit.score, payload })
            })
            .collect();
        Ok(hits)
    }

    async fn delete_document(&self, document_id: &str) -> Result<()> {
        let filter =
            Filter::must([Condition::matches("document_id", document_id.to_owned())]);
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
                    .points(filter)
                    .wait(true),
            )
            .await
            .with_context(|| format!("qdrant delete_document {document_id}"))?;
        tracing::info!(document_id = %document_id, "vettori Qdrant eliminati");
        Ok(())
    }
}
