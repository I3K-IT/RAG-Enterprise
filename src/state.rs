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

/// Stato condiviso clonato in ogni handler axum.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: SqlitePool,
    /// RwLock (non solo Arc): con EmbeddingsSettings::swap_during_ingestion
    /// l'istanza viene sostituita a runtime (CPU↔GPU, vedi
    /// swap_embeddings_to_gpu/_to_cpu) — un semplice Arc non lo permette.
    /// Letture (embed_text/embed_texts) e scritture (swap) avvengono sempre
    /// da contesti sincroni (spawn_blocking), mai attraverso un .await con
    /// il lock preso: std::sync::RwLock è la scelta più semplice, non serve
    /// la variante async di tokio.
    pub embeddings: Arc<RwLock<EmbeddingService>>,
    pub qdrant: Arc<dyn VectorStore>,
    pub eullm: Arc<EullmClient>,
    pub storage: Arc<FileStorage>,
    /// Numero di ingestioni (upload) attualmente nella fase pesante
    /// (parsing+chunking+embedding). Esposto via GET /health così la UI può
    /// mostrare "ingestione in corso" a TUTTI gli utenti connessi, non solo
    /// a chi ha lanciato l'upload — utile già oggi come indicazione, e
    /// diventerà rilevante per davvero quando l'embedding potrà scaricare
    /// temporaneamente eullm dalla VRAM durante l'ingestione (richiede
    /// l'endpoint /api/unload di eullm, non ancora rilasciato).
    pub active_ingestions: Arc<AtomicUsize>,
    /// Some(...) solo se avviato con --bench-live: ogni ingestione/query
    /// reale viene cronometrata e registrata qui invece che scartata — vedi
    /// bench::LiveRecorder. None (default) costa un controllo Option a
    /// richiesta, nessun overhead di misurazione.
    pub live_bench: Option<Arc<LiveRecorder>>,
}

/// Guardia RAII: incrementa active_ingestions alla creazione, decrementa
/// SEMPRE al Drop — anche sui percorsi di errore/return anticipato in
/// upload(), senza doverlo ricordare ad ogni punto di uscita.
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
        embeddings: EmbeddingService,
        qdrant: Arc<dyn VectorStore>,
        eullm: EullmClient,
        live_bench: Option<Arc<LiveRecorder>>,
    ) -> Self {
        let storage = FileStorage::new(&settings.storage.documents_dir);
        Self {
            settings: Arc::new(settings),
            db,
            embeddings: Arc::new(RwLock::new(embeddings)),
            qdrant,
            eullm: Arc::new(eullm),
            storage: Arc::new(storage),
            active_ingestions: Arc::new(AtomicUsize::new(0)),
            live_bench,
        }
    }

    /// true se una query eullm andrebbe rifiutata ora: `unload_during_ingestion`
    /// attivo E almeno un'ingestione in corso (eullm scaricato o in procinto —
    /// vedi documents::upload). Senza questo guard una query concorrente
    /// rifarebbe caricare eullm da sola (stesso meccanismo di swap-on-request
    /// usato per il reload), contendendo la VRAM con l'embedding a metà ingestione.
    pub fn ingestion_blocks_queries(&self) -> bool {
        ingestion_blocks(self.settings.eullm.unload_during_ingestion, &self.active_ingestions)
    }

    /// Sposta bge-m3 su GPU per la finestra di ingestione — vedi
    /// EmbeddingsSettings::swap_during_ingestion. Bloccante (mmap + copia
    /// pesi): il chiamante deve eseguirlo dentro spawn_blocking, mai
    /// direttamente su un task async.
    pub fn swap_embeddings_to_gpu(&self) -> anyhow::Result<()> {
        self.swap_embeddings(EmbeddingService::load_gpu_for_ingestion)
    }

    /// Riporta bge-m3 su CPU a fine ingestione. Stesso vincolo di
    /// blocking di swap_embeddings_to_gpu.
    pub fn swap_embeddings_to_cpu(&self) -> anyhow::Result<()> {
        self.swap_embeddings(EmbeddingService::load_cpu_parked)
    }

    /// Il reload (mmap + copia pesi, potenzialmente qualche secondo) NON
    /// tiene il lock: legge solo model_id sotto un read-lock breve, poi lo
    /// rilascia prima di ricostruire il servizio — le query concorrenti
    /// continuano a usare l'istanza corrente (corretta al momento) invece
    /// di bloccarsi per tutta la durata dello swap.
    fn swap_embeddings(
        &self,
        loader: fn(&str) -> anyhow::Result<EmbeddingService>,
    ) -> anyhow::Result<()> {
        let model_id = {
            let guard = self.embeddings.read().map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
            guard.model_id().to_owned()
        };
        let fresh = loader(&model_id).context("reload embedding su nuovo device")?;
        let mut guard = self.embeddings.write().map_err(|_| anyhow::anyhow!("embeddings: lock poisoned"))?;
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
