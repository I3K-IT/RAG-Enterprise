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

use anyhow::Result;
use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, Distance, SearchPointsBuilder,
        VectorParamsBuilder, DeletePointsBuilder,
        vectors_config::Config, VectorsConfig, Filter, Condition,
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

    /// Upsert chunks in batches of 1000.
    /// TODO Fase 1: implementazione completa con payload serializzato
    pub async fn upsert(&self, _embeddings: &[Vec<f32>], _payloads: &[ChunkPayload]) -> Result<()> {
        todo!("upsert — Fase 1: costruzione PointStruct + UpsertPointsBuilder")
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
