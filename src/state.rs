use std::sync::Arc;
use sqlx::SqlitePool;

use crate::clients::embeddings::EmbeddingService;
use crate::clients::eullm::EullmClient;
use crate::clients::qdrant_store::QdrantStore;
use crate::config::Settings;
use crate::documents::storage::FileStorage;

/// Shared application state cloned into every axum handler.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: SqlitePool,
    pub embeddings: Arc<EmbeddingService>,
    pub qdrant: Arc<QdrantStore>,
    pub eullm: Arc<EullmClient>,
    pub storage: Arc<FileStorage>,
}

impl AppState {
    pub fn new(
        settings: Settings,
        db: SqlitePool,
        embeddings: EmbeddingService,
        qdrant: QdrantStore,
        eullm: EullmClient,
    ) -> Self {
        let storage = FileStorage::new(&settings.storage.documents_dir);
        Self {
            settings: Arc::new(settings),
            db,
            embeddings: Arc::new(embeddings),
            qdrant: Arc::new(qdrant),
            eullm: Arc::new(eullm),
            storage: Arc::new(storage),
        }
    }
}
