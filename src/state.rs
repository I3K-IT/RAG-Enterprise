use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use sqlx::SqlitePool;

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
    pub embeddings: Arc<EmbeddingService>,
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
    ) -> Self {
        let storage = FileStorage::new(&settings.storage.documents_dir);
        Self {
            settings: Arc::new(settings),
            db,
            embeddings: Arc::new(embeddings),
            qdrant,
            eullm: Arc::new(eullm),
            storage: Arc::new(storage),
            active_ingestions: Arc::new(AtomicUsize::new(0)),
        }
    }
}
