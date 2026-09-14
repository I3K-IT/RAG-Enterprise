use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use anyhow::Context;
use sqlx::SqlitePool;

use crate::bench::LiveRecorder;
use crate::clients::embeddings::EmbeddingService;
use crate::clients::eullm::EullmClient;
use crate::auth::throttle::LoginThrottle;
use crate::config::Settings;
use crate::documents::storage::FileStorage;
use crate::extensions::ExtensionRegistry;
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
    /// Extension points the Pro binary can register without forking this
    /// crate — see extensions::ExtensionRegistry. Always
    /// ExtensionRegistry::default() in the Community binary itself.
    pub extensions: Arc<ExtensionRegistry>,
    /// Guards POST /api/auth/login, the one endpoint that does expensive work
    /// (an Argon2 verification) before knowing who is calling — see
    /// auth::throttle for why it throttles by username and total concurrency
    /// rather than by client IP.
    pub login_throttle: Arc<LoginThrottle>,
    /// One permit: the ingestion window (unload → embed → reload) runs one at
    /// a time. See IngestionGuard::start for what happens when it does not.
    pub ingestion_slot: Arc<Semaphore>,
}

/// RAII for the active_ingestions count on its own.
///
/// Its own type, and not just a `fetch_add` inside IngestionGuard::start,
/// because the count has to be claimed BEFORE the wait for the slot — and a
/// bare increment before an `.await` is a leak waiting to happen. A client
/// that disconnects while its upload is queued has axum drop the handler
/// future mid-await; nothing would ever undo that increment, and
/// ingestion_blocks_queries() would answer true for the life of the process,
/// rejecting every query until someone restarted it. Owning the decrement in
/// a value that already exists before the await makes cancellation give the
/// count back for free.
struct CountedIngestion(Arc<AtomicUsize>);

impl CountedIngestion {
    fn start(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for CountedIngestion {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// RAII guard: holds the one ingestion permit and keeps active_ingestions
/// above zero for as long as it lives, ALWAYS releasing both on Drop —
/// including on the error and early-return paths in upload(), so no exit
/// point has to remember to do it.
pub struct IngestionGuard {
    /// Both fields are held, never read; dropping them is the whole point.
    /// Their declaration order is not load-bearing — see start() for why the
    /// count cannot reach zero during a handoff either way.
    _counted: CountedIngestion,
    /// Dropping this is what lets the next upload in.
    _permit: OwnedSemaphorePermit,
}

impl IngestionGuard {
    /// Counts this ingestion, then waits for the slot — in that order.
    ///
    /// The window the slot guards is unload eullm → move bge-m3 onto the GPU
    /// → parse, chunk, embed → move it back → reload eullm, and running two
    /// of those at once breaks in a way that looks like nothing at all:
    /// upload A finishes and reloads the chat model into VRAM while upload B
    /// is still embedding on the GPU, B hits CUDA OOM, falls back to the CPU
    /// and finishes an order of magnitude slower, having reported no error to
    /// anyone. Serialised, the two uploads take about as long as they did
    /// before and neither degrades.
    ///
    /// The order matters as much as the serialising does. Counting only once
    /// the slot is won leaves a gap: A's guard drops and decrements to zero,
    /// and B is still parked inside acquire_owned() and has counted nothing,
    /// so for that instant active_ingestions reads zero on a multithreaded
    /// runtime and a concurrent /api/query sails past
    /// ingestion_blocks_queries() — straight into the upload that was next in
    /// the queue unloading eullm underneath it. Counted first, a queued
    /// upload is an ingestion in progress from the moment it arrives, which
    /// is also what a user watching "ingestion in progress" would expect, and
    /// what the counter meant before there was a queue at all.
    ///
    /// The second upload does wait here with its request body already in
    /// hand — the trade is a queued HTTP request against two ingestions
    /// fighting over the same card, and the queue is the better half of it.
    ///
    /// A Result, not an expect(): this runs at the top of a live ingestion
    /// request, and a closed semaphore must surface as a normal error, not
    /// panic the request task — the same rule as the fallible swap below.
    pub async fn start(counter: &Arc<AtomicUsize>, slot: &Arc<Semaphore>) -> anyhow::Result<Self> {
        let counted = CountedIngestion::start(counter);
        let permit = slot
            .clone()
            .acquire_owned()
            .await
            .context("ingestion slot semaphore is closed")?;
        Ok(Self {
            _counted: counted,
            _permit: permit,
        })
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
        extensions: ExtensionRegistry,
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
            extensions: Arc::new(extensions),
            login_throttle: Arc::new(LoginThrottle::new()),
            ingestion_slot: Arc::new(Semaphore::new(1)),
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

    /// Waits until `counter` reaches `n`, so the test observes the queued
    /// upload having registered rather than guessing at a delay.
    ///
    /// Bounded, not a bare spin. If the count is ever taken after the slot
    /// again instead of before it, a queued upload never registers at all
    /// and an unbounded loop would hang the test — which in CI reads as a
    /// job that timed out, not as the regression it actually is.
    async fn wait_for(counter: &Arc<AtomicUsize>, n: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while counter.load(Ordering::SeqCst) < n {
            assert!(
                std::time::Instant::now() < deadline,
                "active_ingestions never reached {n}: a queued upload is not counting \
                 itself, so the count drops to zero between two uploads"
            );
            tokio::task::yield_now().await;
        }
    }

    /// The regression this ordering exists for. With the count taken after
    /// the slot instead of before it, the assertion right after `drop(a)`
    /// reads zero: A has given the count back and B, still parked inside
    /// acquire_owned(), has not taken one. A query arriving in that instant
    /// passes ingestion_blocks_queries() and then has eullm unloaded under
    /// it by the upload that was next in line.
    #[tokio::test]
    async fn a_queued_upload_holds_the_count_through_the_handoff() {
        let counter = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(Semaphore::new(1));

        let a = IngestionGuard::start(&counter, &slot).await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        let (c, s) = (counter.clone(), slot.clone());
        let queued = tokio::spawn(async move { IngestionGuard::start(&c, &s).await.unwrap() });
        wait_for(&counter, 2).await;

        // Single-threaded test runtime: dropping A cannot yield, so B has
        // provably not woken up yet when this reads the counter.
        drop(a);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "the count must not dip to zero while an upload is still queued"
        );

        let b = queued.await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        drop(b);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    /// What CountedIngestion is a separate type for: a client that hangs up
    /// while its upload is queued has axum drop the handler future
    /// mid-await. A plain increment before that await would never be undone,
    /// and ingestion_blocks_queries() would answer true until restart.
    #[tokio::test]
    async fn abandoning_a_queued_upload_gives_the_count_back() {
        let counter = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(Semaphore::new(1));

        let a = IngestionGuard::start(&counter, &slot).await.unwrap();
        let (c, s) = (counter.clone(), slot.clone());
        let queued = tokio::spawn(async move { IngestionGuard::start(&c, &s).await.unwrap() });
        wait_for(&counter, 2).await;

        queued.abort();
        let _ = queued.await;
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "an abandoned queued upload must not leak its count"
        );

        drop(a);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

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

    /// A closed ingestion slot must surface as an error, not panic the
    /// request task — however unreachable closing it is in practice.
    #[tokio::test]
    async fn closed_slot_is_an_error_not_a_panic() {
        let counter = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(Semaphore::new(1));
        slot.close();
        assert!(IngestionGuard::start(&counter, &slot).await.is_err());
    }
}
