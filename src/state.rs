use std::sync::Arc;
use sqlx::SqlitePool;

use crate::clients::embeddings::EmbeddingService;
use crate::clients::qdrant_store::QdrantStore;
use crate::config::Settings;

/// Shared application state cloned into every axum handler.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: SqlitePool,
    pub embeddings: Arc<EmbeddingService>,
    pub qdrant: Arc<QdrantStore>,
}

impl AppState {
    pub fn new(
        settings: Settings,
        db: SqlitePool,
        embeddings: EmbeddingService,
        qdrant: QdrantStore,
    ) -> Self {
        Self {
            settings: Arc::new(settings),
            db,
            embeddings: Arc::new(embeddings),
            qdrant: Arc::new(qdrant),
        }
    }
}
