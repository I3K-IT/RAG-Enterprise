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
//!
//! Searches go through the Query API: the older Search endpoints are
//! deprecated, and Qdrant's 1.19 release notes announce their removal. With
//! QDRANT__QUANTIZATION set, the collection also keeps a TurboQuant copy of
//! its vectors in RAM — see ensure_quantization.

use anyhow::{Context, Result};
use async_trait::async_trait;
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder,
        DeletePointsBuilder, Disabled, Distance, FieldType, Filter, PointStruct,
        QuantizationSearchParamsBuilder, QueryPointsBuilder, ScrollPointsBuilder,
        SearchParamsBuilder,
        TurboQuantBitSize, TurboQuantization, TurboQuantizationBuilder,
        UpdateCollectionBuilder, UpsertPointsBuilder, VectorParamsBuilder,
        VectorParamsDiffBuilder, VectorsConfig, VectorsConfigDiff, quantization_config,
        quantization_config_diff, vectors_config::Config, vectors_config_diff,
    },
};

use crate::config::VectorQuantization;
use crate::rag::vector_store::{ChunkPayload, SearchHit, VectorStore};

pub const VECTOR_DIM: u64 = 1024;

/// The one payload field this code filters on — see ensure_document_id_index.
const DOCUMENT_ID_FIELD: &str = "document_id";

pub struct QdrantStore {
    client: Qdrant,
    collection: String,
    quantization: VectorQuantization,
}

impl QdrantStore {
    pub async fn new(
        url: &str,
        collection: &str,
        quantization: VectorQuantization,
    ) -> Result<Self> {
        let client = Qdrant::from_url(url).build()?;
        let store = Self { client, collection: collection.to_owned(), quantization };
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
        self.ensure_quantization().await;
        Ok(())
    }

    /// Brings the collection in line with QDRANT__QUANTIZATION: TurboQuant
    /// with the original vectors on disk, or neither. Run at every startup,
    /// like the index above, so it also converts a collection an earlier
    /// version created — and converts it back when the setting is turned off.
    /// Qdrant re-encodes the vectors in the background, and searches keep
    /// working meanwhile.
    ///
    /// Only TurboQuant is ever undone: a collection quantized another way
    /// was set up so by hand, and "off" leaves it alone.
    ///
    /// Logs rather than propagates, for the same reason as the index: how
    /// much RAM the vectors take is not whether the engine works. Returns
    /// whether it asked Qdrant to change the collection.
    async fn ensure_quantization(&self) -> bool {
        let wanted = turbo_for(self.quantization);
        let current = match self.client.collection_info(&self.collection).await {
            Ok(info) => info.result.and_then(|info| info.config),
            Err(e) => {
                tracing::warn!(
                    collection = %self.collection,
                    error = %e,
                    "Qdrant quantization not checked: could not read the collection"
                );
                return false;
            }
        };
        let quantization = current
            .as_ref()
            .and_then(|config| config.quantization_config)
            .and_then(|config| config.quantization);
        let originals_on_disk = current
            .and_then(|config| config.params)
            .and_then(|params| params.vectors_config)
            .and_then(|vectors| match vectors.config {
                Some(Config::Params(params)) => params.on_disk,
                _ => None,
            })
            .unwrap_or(false);
        let turbo = match quantization {
            Some(quantization_config::Quantization::Turboquant(turbo)) => Some(turbo),
            Some(_) if wanted.is_none() => return false,
            _ => None,
        };
        let in_place = match (&wanted, &turbo) {
            (None, None) => true,
            (Some(wanted), Some(turbo)) => same_turbo(wanted, turbo) && originals_on_disk,
            _ => false,
        };
        if in_place {
            return false;
        }
        let update = UpdateCollectionBuilder::new(&self.collection)
            .vectors_config(VectorsConfigDiff::from(vectors_config_diff::Config::from(
                VectorParamsDiffBuilder::default().on_disk(wanted.is_some()),
            )))
            .quantization_config(match wanted {
                Some(turbo) => quantization_config_diff::Quantization::Turboquant(turbo),
                None => quantization_config_diff::Quantization::Disabled(Disabled {}),
            });
        match self.client.update_collection(update).await {
            Ok(_) => tracing::info!(
                collection = %self.collection,
                quantization = ?self.quantization,
                "Qdrant vectors re-encoding in the background"
            ),
            Err(e) => tracing::warn!(
                collection = %self.collection,
                quantization = ?self.quantization,
                error = %e,
                "Qdrant quantization not applied (TurboQuant needs Qdrant 1.18 or later)"
            ),
        }
        true
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
        let mut builder = QueryPointsBuilder::new(&self.collection)
            .query(query_vec)
            .limit(top_k)
            .with_payload(true);
        if let Some(t) = score_threshold {
            builder = builder.score_threshold(t);
        }
        if let Some(oversampling) = oversampling_for(self.quantization) {
            // Pick the candidates on the compressed vectors, then order them
            // by the originals. Only that shortlist is re-scored: a chunk
            // the compression ranks below it is missed, the price of the RAM
            // saved (see VectorQuantization).
            builder = builder.params(
                SearchParamsBuilder::default().quantization(
                    QuantizationSearchParamsBuilder::default()
                        .rescore(true)
                        .oversampling(oversampling),
                ),
            );
        }
        let resp = self.client.query(builder).await?;
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

    /// A filtered scroll on `document_id`, which has a payload index (see
    /// ensure_document_id_index), so it reads only that document's points.
    async fn document_chunks(&self, document_id: &str, limit: u64) -> Result<Vec<SearchHit>> {
        let filter =
            Filter::must([Condition::matches(DOCUMENT_ID_FIELD, document_id.to_owned())]);
        let resp = self
            .client
            .scroll(
                ScrollPointsBuilder::new(&self.collection)
                    .filter(filter)
                    .limit(limit.min(u32::MAX as u64) as u32)
                    .with_payload(true),
            )
            .await
            .with_context(|| format!("qdrant document_chunks {document_id}"))?;
        let mut hits: Vec<SearchHit> = resp
            .result
            .into_iter()
            .filter_map(|point| {
                let raw: serde_json::Map<String, serde_json::Value> =
                    point.payload.into_iter().map(|(k, v)| (k, v.into())).collect();
                match serde_json::from_value::<ChunkPayload>(serde_json::Value::Object(raw)) {
                    Ok(payload) => Some(SearchHit { similarity: 1.0, payload }),
                    Err(e) => {
                        tracing::warn!(
                            document_id = %document_id,
                            error = %e,
                            "Qdrant: payload will not deserialize, chunk excluded"
                        );
                        None
                    }
                }
            })
            .collect();
        hits.sort_by_key(|h| h.payload.chunk_index);
        Ok(hits)
    }
}

/// The TurboQuant configuration a setting asks for, if any. The compressed
/// vectors stay in RAM; the originals go to disk.
fn turbo_for(quantization: VectorQuantization) -> Option<TurboQuantization> {
    let bits = match quantization {
        VectorQuantization::Off => return None,
        VectorQuantization::Turbo4 => TurboQuantBitSize::Bits4,
        VectorQuantization::Turbo2 => TurboQuantBitSize::Bits2,
    };
    Some(TurboQuantizationBuilder::default().bits(bits).always_ram(true).build())
}

/// Whether a collection's TurboQuant is the one wanted. Qdrant leaves out
/// the fields at their default: 4 bits, RAM following the vectors.
fn same_turbo(wanted: &TurboQuantization, current: &TurboQuantization) -> bool {
    let bits = |t: &TurboQuantization| t.bits.unwrap_or(TurboQuantBitSize::Bits4 as i32);
    bits(wanted) == bits(current) && wanted.always_ram == current.always_ram
}

/// How many candidates per result each search re-scores against the
/// original vectors: the fewer the bits, the more it takes to stay close to
/// a full-precision search. Close, not equal — see VectorQuantization.
fn oversampling_for(quantization: VectorQuantization) -> Option<f64> {
    match quantization {
        VectorQuantization::Off => None,
        VectorQuantization::Turbo4 => Some(2.0),
        VectorQuantization::Turbo2 => Some(4.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qdrant_client::qdrant::{CollectionStatus, OptimizersConfigDiffBuilder};

    #[test]
    fn each_setting_maps_to_its_turboquant() {
        assert!(turbo_for(VectorQuantization::Off).is_none());
        let bits = |q| turbo_for(q).and_then(|t| t.bits);
        assert_eq!(bits(VectorQuantization::Turbo4), Some(TurboQuantBitSize::Bits4 as i32));
        assert_eq!(bits(VectorQuantization::Turbo2), Some(TurboQuantBitSize::Bits2 as i32));
        assert_eq!(turbo_for(VectorQuantization::Turbo4).unwrap().always_ram, Some(true));
        assert_eq!(oversampling_for(VectorQuantization::Off), None);
        assert!(
            oversampling_for(VectorQuantization::Turbo2)
                > oversampling_for(VectorQuantization::Turbo4)
        );
    }

    #[test]
    fn bits_left_out_count_as_four() {
        let reported = TurboQuantization { bits: None, always_ram: Some(true) };
        assert!(same_turbo(&turbo_for(VectorQuantization::Turbo4).unwrap(), &reported));
        assert!(!same_turbo(&turbo_for(VectorQuantization::Turbo2).unwrap(), &reported));
        let following_the_vectors = TurboQuantization { bits: None, always_ram: None };
        let turbo4 = turbo_for(VectorQuantization::Turbo4).unwrap();
        assert!(!same_turbo(&turbo4, &following_the_vectors));
    }

    // ── against a real Qdrant ─────────────────────────────────────────────────

    /// Ten documents of forty chunks, each document around its own
    /// direction, chunk k drifting further from it as k grows: a query at a
    /// document's direction must find its chunks 0 to 4, in that order.
    /// Clear gaps on purpose — near-ties are what quantization may reorder
    /// or drop, which is accepted, so here any difference is a bug.
    fn corpus() -> (Vec<Vec<f32>>, Vec<ChunkPayload>, Vec<Vec<f32>>) {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        let normalize = |v: Vec<f32>| {
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.into_iter().map(|x| x / norm).collect::<Vec<f32>>()
        };
        let centers: Vec<Vec<f32>> = (0..10)
            .map(|_| normalize((0..VECTOR_DIM).map(|_| noise()).collect()))
            .collect();
        let (mut vectors, mut payloads) = (Vec::new(), Vec::new());
        for (doc, center) in centers.iter().enumerate() {
            for chunk in 0..40 {
                let drift = 0.01 * (chunk + 1) as f32;
                vectors.push(normalize(center.iter().map(|x| x + noise() * drift).collect()));
                payloads.push(ChunkPayload {
                    document_id: format!("doc-{doc}"),
                    chunk_index: chunk,
                    filename: format!("doc-{doc}.txt"),
                    upload_date: "2026-09-27T00:00:00Z".into(),
                    text: format!("chunk {chunk} of document {doc}"),
                    chunk_size: 20,
                    document_type: "txt".into(),
                    structured_fields: None,
                    source_start_byte: None,
                    source_end_byte: None,
                    page_start: None,
                    page_end: None,
                    provenance_id: None,
                    retrieval_text: None,
                });
            }
        }
        (vectors, payloads, centers)
    }

    async fn settle(client: &Qdrant, collection: &str) {
        for _ in 0..120 {
            let info = client.collection_info(collection).await.unwrap().result.unwrap();
            if info.status == CollectionStatus::Green as i32 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        panic!("{collection} never finished optimizing");
    }

    async fn layout(client: &Qdrant, collection: &str) -> (Option<TurboQuantization>, bool) {
        let info = client.collection_info(collection).await.unwrap().result.unwrap();
        let config = info.config.unwrap();
        let turbo = match config.quantization_config.and_then(|q| q.quantization) {
            Some(quantization_config::Quantization::Turboquant(turbo)) => Some(turbo),
            _ => None,
        };
        let on_disk = match config.params.unwrap().vectors_config.unwrap().config {
            Some(Config::Params(params)) => params.on_disk.unwrap_or(false),
            _ => false,
        };
        (turbo, on_disk)
    }

    fn found(hits: &[SearchHit]) -> Vec<(String, usize)> {
        hits.iter().map(|h| (h.payload.document_id.clone(), h.payload.chunk_index)).collect()
    }

    /// The Query API, TurboQuant switched on and back off on a live
    /// collection, a clear ranking coming out the same either way, and a
    /// restart with the same setting leaving the collection alone.
    ///
    ///   QDRANT_GRPC_URL_FOR_TEST=http://localhost:6334 \
    ///     cargo test turboquant_on_and_off -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a running Qdrant — set QDRANT_GRPC_URL_FOR_TEST"]
    async fn a_collection_searches_the_same_with_turboquant_on_and_off() {
        let Ok(url) = std::env::var("QDRANT_GRPC_URL_FOR_TEST") else {
            panic!("set QDRANT_GRPC_URL_FOR_TEST to the gRPC URL of a running Qdrant");
        };
        let coll = "qdrant_store_turboquant_on_and_off";
        let client = Qdrant::from_url(&url).build().unwrap();
        if client.collection_exists(coll).await.unwrap() {
            client.delete_collection(coll).await.unwrap();
        }
        // A low indexing threshold, so these few hundred points get the
        // indexed, quantized segments a real collection has.
        client
            .create_collection(
                CreateCollectionBuilder::new(coll)
                    .vectors_config(VectorParamsBuilder::new(VECTOR_DIM, Distance::Cosine))
                    .optimizers_config(
                        OptimizersConfigDiffBuilder::default().indexing_threshold(10),
                    ),
            )
            .await
            .unwrap();

        let (vectors, payloads, queries) = corpus();
        let store = QdrantStore::new(&url, coll, VectorQuantization::Off).await.unwrap();
        store.upsert(&vectors, &payloads).await.unwrap();
        settle(&client, coll).await;
        assert_eq!(layout(&client, coll).await, (None, false));
        assert!(!store.ensure_quantization().await, "off on a plain collection changes nothing");
        let expected = |doc: usize| {
            (0..5).map(|chunk| (format!("doc-{doc}"), chunk)).collect::<Vec<_>>()
        };
        for (doc, query) in queries.iter().enumerate() {
            let hits = store.search(query.clone(), 5, Some(0.5)).await.unwrap();
            assert_eq!(found(&hits), expected(doc));
        }

        for quantization in [VectorQuantization::Turbo4, VectorQuantization::Turbo2] {
            let store = QdrantStore::new(&url, coll, quantization).await.unwrap();
            settle(&client, coll).await;
            let applied = (turbo_for(quantization), true);
            assert_eq!(layout(&client, coll).await, applied, "{quantization:?}");
            let again = store.ensure_quantization().await;
            assert!(!again, "{quantization:?}: a restart changes nothing");
            for (doc, query) in queries.iter().enumerate() {
                let hits = store.search(query.clone(), 5, Some(0.5)).await.unwrap();
                assert_eq!(found(&hits), expected(doc), "{quantization:?}");
            }
        }

        let store = QdrantStore::new(&url, coll, VectorQuantization::Off).await.unwrap();
        settle(&client, coll).await;
        assert_eq!(layout(&client, coll).await, (None, false), "turned off again");
        assert!(!store.ensure_quantization().await, "and a restart changes nothing");

        store.delete_document("doc-3").await.unwrap();
        let hits = store.search(queries[3].clone(), 5, None).await.unwrap();
        assert!(hits.iter().all(|h| h.payload.document_id != "doc-3"), "{:?}", found(&hits));

        client.delete_collection(coll).await.unwrap();
    }
}
