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
        }
    }
}
