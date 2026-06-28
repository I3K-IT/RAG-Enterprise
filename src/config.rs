use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub server: ServerSettings,
    pub database: DatabaseSettings,
    pub auth: AuthSettings,
    pub qdrant: QdrantSettings,
    pub eullm: EullmSettings,
    pub embeddings: EmbeddingsSettings,
    pub backup: BackupSettings,
}

#[derive(Debug, Deserialize)]
pub struct ServerSettings {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseSettings {
    #[serde(default = "default_db_url")]
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthSettings {
    pub jwt_secret: String,
    #[serde(default = "default_jwt_expiry")]
    pub jwt_expiry_minutes: u64,
    pub admin_default_password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QdrantSettings {
    #[serde(default = "default_qdrant_url")]
    pub url: String,
    #[serde(default = "default_collection")]
    pub collection: String,
}

#[derive(Debug, Deserialize)]
pub struct EullmSettings {
    #[serde(default = "default_eullm_url")]
    pub url: String,
    pub model: String,
    #[serde(default = "default_num_ctx")]
    pub num_ctx: u32,
    #[serde(default = "default_num_predict")]
    pub num_predict: u32,
    #[serde(default = "default_repeat_penalty")]
    pub repeat_penalty: f32,
    #[serde(default = "default_keep_alive")]
    pub keep_alive: i32,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsSettings {
    #[serde(default = "default_embedding_model")]
    pub model_id: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupSettings {
    #[serde(default = "default_backup_dir")]
    pub dir: String,
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8000 }
fn default_db_url() -> String { "sqlite://rag_users.db".into() }
fn default_jwt_expiry() -> u64 { 480 }
fn default_qdrant_url() -> String { "http://localhost:6333".into() }
fn default_collection() -> String { "rag_documents".into() }
fn default_eullm_url() -> String { "http://localhost:11434".into() }
fn default_num_ctx() -> u32 { 16384 }
fn default_num_predict() -> u32 { 4096 }
fn default_repeat_penalty() -> f32 { 1.3 }
fn default_keep_alive() -> i32 { -1 }
fn default_embedding_model() -> String { "BAAI/bge-m3".into() }
fn default_backup_dir() -> String { "./backups".into() }

impl Settings {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();
        let cfg = ::config::Config::builder()
            .add_source(::config::Environment::default().separator("__"))
            .build()?;
        Ok(cfg.try_deserialize()?)
    }
}
