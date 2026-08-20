use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use anyhow::Context;
use sqlx::SqlitePool;

use crate::bench::LiveRecorder;
use crate::clients::embeddings::EmbeddingService;
use crate::clients::eullm::EullmClient;
use crate::config::Settings;
use crate::documents::storage::FileStorage;
use crate::rag::vector_store::VectorStore;

/// Shared state, cloned into every axum handler.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: SqlitePool,
    /// None exactly when ingestion_embedding=Eullm: in that mode BOTH
    /// ingestion and query embedding route through AppState.eullm instead
    /// (see api/documents.rs and api/query.rs::prepare), so Candle is never
    /// loaded at all — not even on CPU — and the ~3.2GB of bge-m3 weights
    /// (Candle .safetensors) never needs downloading either, see
    /// bootstrap::select_components. Every other call site that reaches into
    /// this field is already behind an `ingestion_embedding != Eullm` check
    /// (or, for the CandleGpu swap functions below, only reachable when
    /// ingestion_embedding=CandleGpu specifically) and may assume Some.
    ///
    /// An RwLock, not merely an Arc, inside the Some: with
    /// EmbeddingsSettings::ingestion_embedding=CandleGpu the instance is
    /// replaced at runtime (CPU↔GPU, see swap_embeddings_to_gpu/_to_cpu),
    /// which a plain Arc does not allow. Reads (embed_text/embed_texts) and
    /// writes (the swap) always happen from synchronous contexts inside
    /// spawn_blocking, never across an .await while holding the lock, so
    /// std::sync::RwLock is the simplest choice and tokio's async variant is
    /// unnecessary.
    pub embeddings: Option<Arc<RwLock<EmbeddingService>>>,
    pub qdrant: Arc<dyn VectorStore>,
    pub eullm: Arc<EullmClient>,
    pub storage: Arc<FileStorage>,
    /// How many ingestions (uploads) are currently in the heavy phase of
    /// parsing, chunking and embedding. Exposed through GET /health so the UI
    /// can show "ingestion in progress" to ALL connected users, not only to
    /// whoever started the upload.
    pub active_ingestions: Arc<AtomicUsize>,
    /// Some(...) only when started with --bench-live: every real ingestion and
    /// query is timed and recorded here instead of discarded (see
    /// bench::LiveRecorder). None, the default, costs one Option check per
    /// request and no measurement overhead.
    pub live_bench: Option<Arc<LiveRecorder>>,
}

/// RAII guard: increments active_ingestions on creation and ALWAYS decrements
/// on Drop, including on the error and early-return paths in upload(), so no
/// exit point has to remember to do it.
pub struct IngestionGuard(Arc<AtomicUsize>);

impl IngestionGuard {
    pub fn start(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for IngestionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AppState {
    pub fn new(
        settings: Settings,
        db: SqlitePool,
        embeddings: Option<EmbeddingService>,
        qdrant: Arc<dyn VectorStore>,
        eullm: EullmClient,
        live_bench: Option<Arc<LiveRecorder>>,
    ) -> Self {
        let storage = FileStorage::new(&settings.storage.documents_dir);
        Self {
            settings: Arc::new(settings),
            db,
            embeddings: embeddings.map(|e| Arc::new(RwLock::new(e))),
            qdrant,
            eullm: Arc::new(eullm),
            storage: Arc::new(storage),
            active_ingestions: Arc::new(AtomicUsize::new(0)),
            live_bench,
        }
    }

    /// True when an eullm query should be rejected right now: at least one
    /// ingestion is in flight AND eullm is evicted, about to be, or
    /// otherwise contended for — either because `unload_during_ingestion`
    /// evicts it ourselves (see documents::upload), or because
    /// `ingestion_embedding=Eullm` is actively asking eullm for bge-m3
    /// embeddings, which may make eullm evict its own chat model to make
    /// room (its decision, not ours — see config::IngestionEmbedding::Eullm).
    /// Without this guard a concurrent query would load/contend for eullm
    /// again by itself, through the same swap-on-request mechanism used for
    /// the reload, racing the ingestion's own use of it.
    pub fn ingestion_blocks_queries(&self) -> bool {
        let blocks_regardless = self.settings.eullm.unload_during_ingestion
            || self.settings.embeddings.ingestion_embedding
                == crate::config::IngestionEmbedding::Eullm;
        ingestion_blocks(blocks_regardless, &self.active_ingestions)
    }

    /// Moves bge-m3 onto the GPU for the ingestion window — see
    /// config::IngestionEmbedding::CandleGpu. Blocking (mmap plus weight
    /// copy): the caller must run it inside spawn_blocking, never directly on
    /// an async task.
    pub fn swap_embeddings_to_gpu(&self) -> anyhow::Result<()> {
        self.swap_embeddings(EmbeddingService::load_gpu_for_ingestion)
    }

    /// Moves bge-m3 back onto the CPU once ingestion ends. Same blocking
    /// constraint as swap_embeddings_to_gpu.
    pub fn swap_embeddings_to_cpu(&self) -> anyhow::Result<()> {
        self.swap_embeddings(EmbeddingService::load_cpu_parked)
    }

    /// The reload (mmap plus weight copy, potentially a few seconds) does NOT
    /// hold the lock: it reads only model_id under a brief read lock, then
    /// releases it before rebuilding the service. Concurrent queries keep
    /// using the current instance — correct at that moment — instead of
    /// blocking for the whole duration of the swap.
    fn swap_embeddings(
        &self,
        loader: fn(&str) -> anyhow::Result<EmbeddingService>,
    ) -> anyhow::Result<()> {
        // Only reachable when ingestion_embedding=CandleGpu (see
        // api/documents.rs's candle_gpu gate), and Candle is always loaded
        // in that mode — see the doc comment on `embeddings`. A Result, not
        // an expect(): this runs inside a live ingestion request, and a
        // config/code mismatch should surface as a normal error, not panic
        // the request task.
        let embeddings = self
            .embeddings
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("swap_embeddings called without ingestion_embedding=CandleGpu"))?;
        let model_id = {
            let guard = embeddings.read().map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
            guard.model_id().to_owned()
        };
        let fresh = loader(&model_id).context("reload embedding su nuovo device")?;
        let mut guard = embeddings.write().map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
        *guard = fresh;
        Ok(())
    }
}

fn ingestion_blocks(unload_during_ingestion: bool, active_ingestions: &AtomicUsize) -> bool {
    unload_during_ingestion && active_ingestions.load(Ordering::SeqCst) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingestion_blocks_false_when_feature_disabled() {
        let counter = AtomicUsize::new(1);
        assert!(!ingestion_blocks(false, &counter));
    }

    #[test]
    fn ingestion_blocks_false_when_idle() {
        let counter = AtomicUsize::new(0);
        assert!(!ingestion_blocks(true, &counter));
    }

    #[test]
    fn ingestion_blocks_true_when_enabled_and_active() {
        let counter = AtomicUsize::new(1);
        assert!(ingestion_blocks(true, &counter));
    }
}
