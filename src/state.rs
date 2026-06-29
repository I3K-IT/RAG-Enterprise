use std::sync::Arc;
use sqlx::SqlitePool;

use crate::config::Settings;

/// Shared application state, cloned into every axum handler.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: SqlitePool,
    // Added in Fase 1 when clients are implemented:
    // pub qdrant: Arc<clients::qdrant::QdrantClient>,
    // pub embeddings: Arc<clients::embeddings::EmbeddingService>,
    // pub eullm: Arc<clients::eullm::EullmClient>,
}

impl AppState {
    pub fn new(settings: Settings, db: SqlitePool) -> Self {
        Self {
            settings: Arc::new(settings),
            db,
        }
    }
}
