use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub server: ServerSettings,
    #[serde(default)]
    pub database: DatabaseSettings,
    pub auth: AuthSettings,          // jwt_secret required — no default
    #[serde(default)]
    pub qdrant: QdrantSettings,
    pub eullm: EullmSettings,        // model required — no default
    #[serde(default)]
    pub embeddings: EmbeddingsSettings,
    #[serde(default)]
    pub backup: BackupSettings,
    #[serde(default)]
    pub storage: StorageSettings,
    #[serde(default)]
    pub data: DataSettings,
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
    /// REST endpoint (HTTP/1.1) — usato per healthcheck in bootstrap.
    #[serde(default = "default_qdrant_url")]
    pub url: String,
    /// gRPC endpoint (HTTP/2, tonic) — usato da QdrantStore (qdrant-client).
    #[serde(default = "default_qdrant_grpc_url")]
    pub grpc_url: String,
    #[serde(default = "default_collection")]
    pub collection: String,
}

#[derive(Debug, Deserialize)]
pub struct EullmSettings {
    #[serde(default = "default_eullm_url")]
    pub url: String,
    /// Usato solo se manage_subprocesses=false (eullm gestito esternamente):
    /// deve essere un path GGUF diretto o un nome importato via
    /// `eullm import-ollama` — un nome "sciolto" (es. "qwen3:14b") senza
    /// nessuna delle due cose fallisce con 500 "model not found". Quando
    /// bootstrap avvia eullm da sé, questo valore viene ignorato: si usa
    /// automaticamente lo stesso path GGUF passato a `eullm run` (vedi
    /// bootstrap::ProcessGuard::eullm_model_path e main.rs).
    pub model: String,
    /// Contesto PER CONNESSIONE (slot). Il flag di avvio --ctx-size passato a
    /// eullm è il TOTALE: num_ctx * batch_size (vedi bootstrap::spawn_eullm).
    #[serde(default = "default_num_ctx")]
    pub num_ctx: u32,
    #[serde(default = "default_num_predict")]
    pub num_predict: u32,
    #[serde(default = "default_repeat_penalty")]
    pub repeat_penalty: f32,
    #[serde(default = "default_keep_alive")]
    pub keep_alive: i32,
    /// Connessioni concorrenti gestite da eullm. VRAM per la KV cache scala
    /// linearmente con num_ctx * batch_size — verifica il budget VRAM prima
    /// di alzarlo (vedi BUILD.md).
    #[serde(default = "default_eullm_batch_size")]
    pub batch_size: u32,
    /// Quantizzazione KV cache (llama.cpp: f16, q8_0, q4_0…). None = default
    /// di eullm (F16, nessun flag passato). Da testare per piattaforma/GPU —
    /// non dare per scontato che sia supportata (verificare nei log di eullm).
    #[serde(default)]
    pub cache_type_k: Option<String>,
    #[serde(default)]
    pub cache_type_v: Option<String>,
    /// Passa --fit a eullm (verificato presente da EuLLM-v0.6.9 — versioni
    /// pinnate precedenti, es. v0.6.6 su x86_64 oggi, NON hanno questo flag:
    /// clap lo rifiuterebbe come argomento sconosciuto. Attivalo solo se il
    /// binario pinnato per la piattaforma lo supporta davvero.
    ///
    /// Con fit=true cambia anche l'ORDINE di avvio (vedi main.rs): l'embedding
    /// carica PRIMA di eullm, così il probe VRAM di --fit (cudaMemGetInfo,
    /// letto a caldo all'avvio di eullm) vede la VRAM già ridotta dal modello
    /// di embedding e adatta di conseguenza quanti layer offloadare su
    /// GPU/CPU — invece di rischiare che i due si contendano la VRAM
    /// all'avvio (vedi audit Fase 1, punto 5a).
    #[serde(default)]
    pub fit: bool,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsSettings {
    #[serde(default = "default_embedding_model")]
    pub model_id: String,
    /// Se true: CUDA obbligatoria per l'embedding. Se l'init CUDA fallisce dopo i
    /// retry (vedi EmbeddingService::load), l'avvio FALLISCE invece di degradare
    /// in silenzio su CPU — un'ingestione a 17 minuti anziché secondi non deve
    /// mai passare inosservata. Default false: fallback CPU consentito ma
    /// loggato a livello error (non warn) ed esposto via GET /info.
    /// Env: EMBEDDINGS__REQUIRE_GPU (il campo Settings si chiama "embeddings").
    #[serde(default)]
    pub require_gpu: bool,
}

#[derive(Debug, Deserialize)]
pub struct BackupSettings {
    #[serde(default = "default_backup_dir")]
    pub dir: String,
}

#[derive(Debug, Deserialize)]
pub struct StorageSettings {
    #[serde(default = "default_documents_dir")]
    pub documents_dir: String,
    /// Limite upload documento (MB). Parità Python: MAX_UPLOAD_SIZE_MB, default 100.
    #[serde(default = "default_max_upload_mb")]
    pub max_upload_mb: u64,
}

/// Radice dati: binari, modelli, storage Qdrant, db SQLite, uploads.
/// Layout: {dir}/bin/  {dir}/models/  {dir}/storage/  {dir}/db/  {dir}/uploads/
#[derive(Debug, Deserialize)]
pub struct DataSettings {
    /// Percorso radice (default: cartella dell'eseguibile — portable app dir).
    /// Override dev: DATA__DIR=/percorso/cartella/lavoro (il binario sta in target/debug/).
    #[serde(default = "default_data_dir")]
    pub dir: String,
    /// Se true: bootstrap avvia e supervisiona qdrant ed eullm come processi figlio.
    /// Se false: si aspetta che i processi siano già in ascolto (dev/compose).
    #[serde(default = "default_manage_subprocesses")]
    pub manage_subprocesses: bool,
}

impl DataSettings {
    pub fn data_path(&self) -> PathBuf {
        expand_tilde(&self.dir)
    }
}

fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" {
        std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
    } else if let Some(rest) = s.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(s)
    }
}

impl Default for ServerSettings {
    fn default() -> Self { Self { host: default_host(), port: default_port() } }
}
impl Default for DatabaseSettings {
    fn default() -> Self { Self { url: default_db_url() } }
}
impl Default for QdrantSettings {
    fn default() -> Self {
        Self {
            url: default_qdrant_url(),
            grpc_url: default_qdrant_grpc_url(),
            collection: default_collection(),
        }
    }
}
impl Default for EmbeddingsSettings {
    fn default() -> Self { Self { model_id: default_embedding_model(), require_gpu: false } }
}
impl Default for BackupSettings {
    fn default() -> Self { Self { dir: default_backup_dir() } }
}
impl Default for StorageSettings {
    fn default() -> Self {
        Self { documents_dir: default_documents_dir(), max_upload_mb: default_max_upload_mb() }
    }
}
impl Default for DataSettings {
    fn default() -> Self {
        Self { dir: default_data_dir(), manage_subprocesses: default_manage_subprocesses() }
    }
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8000 }
fn default_db_url() -> String { "sqlite://rag_users.db".into() }
fn default_jwt_expiry() -> u64 { 480 }
fn default_qdrant_url() -> String { "http://localhost:6333".into() }
fn default_qdrant_grpc_url() -> String { "http://localhost:6334".into() }
fn default_collection() -> String { "rag_documents".into() }
fn default_eullm_url() -> String { "http://localhost:11434".into() }
fn default_num_ctx() -> u32 { 16384 }
fn default_eullm_batch_size() -> u32 { 1 }
fn default_num_predict() -> u32 { 4096 }
fn default_repeat_penalty() -> f32 { 1.3 }
fn default_keep_alive() -> i32 { -1 }
fn default_embedding_model() -> String { "BAAI/bge-m3".into() }
fn default_backup_dir() -> String { "./backups".into() }
fn default_documents_dir() -> String { "./documents".into() }
fn default_max_upload_mb() -> u64 { 100 }
fn default_data_dir() -> String {
    // Exe-relative: la cartella che contiene il binario è la portable app dir.
    // In produzione: /install/dir/i3k-rag-engine → data = /install/dir/
    // In dev (cargo run): target/debug/ → override con DATA__DIR=./data
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .display()
        .to_string()
}
fn default_manage_subprocesses() -> bool { true }

impl Settings {
    pub fn load() -> Result<Self> {
        // Cerca .env prima nella CWD (dev: cargo run dalla root),
        // poi nella cartella dell'eseguibile (produzione: binary dir).
        if dotenvy::dotenv().is_err() {
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    dotenvy::from_path(dir.join(".env")).ok();
                }
            }
        }
        let cfg = ::config::Config::builder()
            .add_source(::config::Environment::default().separator("__"))
            .build()?;
        let mut s: Self = cfg.try_deserialize()?;

        // Deriva i path da data_dir se sono ancora ai valori di default.
        // Permette di impostare solo DATA__DIR per spostare tutto.
        let data = s.data.data_path();
        if s.database.url == default_db_url() {
            s.database.url = format!("sqlite://{}", data.join("db").join("rag_users.db").display());
        }
        if s.storage.documents_dir == default_documents_dir() {
            s.storage.documents_dir = data.join("uploads").display().to_string();
        }
        if s.backup.dir == default_backup_dir() {
            s.backup.dir = data.join("backups").display().to_string();
        }

        Ok(s)
    }
}
