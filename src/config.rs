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
    // NOTA: il campo `fit` è stato rimosso con la pin 0.6.80. Da quella
    // versione eullm dimensiona l'offload GPU da sé, SEMPRE, a prescindere dal
    // flag: `--fit` passato esplicitamente serve solo a far chiedere conferma
    // su uno split parziale, e solo quando stdin E stdout sono entrambi TTY
    // (fit.rs:851 nel sorgente eullm) — noi lanciamo con stdin su null, quindi
    // per noi era un no-op. L'ordine di avvio che il flag governava è ora
    // incondizionato in main.rs. Config esistenti con `fit = ...` continuano a
    // caricare: la struct non usa deny_unknown_fields, il campo viene ignorato.
    /// Scarica eullm dalla VRAM durante l'ingestione documenti (POST
    /// /api/unload — estensione EULLM verificata nel sorgente eullm:
    /// api_routes() registra `.route("/unload", post(unload_model))`, ed è
    /// montata sotto /api). Con questo attivo, la chat non
    /// funziona per la durata dell'ingestione (eullm non è in VRAM) — la UI
    /// mostra "ingestione in corso" ma non blocca l'invio, quindi una
    /// domanda fatta in quella finestra fallirebbe o resterebbe in attesa
    /// fino al reload.
    #[serde(default)]
    pub unload_during_ingestion: bool,
    /// Override del modello da avviare, SOLO quando manage_subprocesses=true
    /// (altrimenti usa già `model` così com'è — vedi sopra). Se Some, bypassa
    /// la ricerca del componente "qwen3-14b" pinnato nel manifest e passa
    /// questo valore direttamente a `eullm run` — path GGUF locale o
    /// riferimento `hf.co/utente/repo:quant` che eullm risolve/scarica da sé,
    /// fuori dal nostro manifest sha256-pinnato: la verifica di integrità in
    /// quel caso è responsabilità di eullm/dell'hub HF, non nostra.
    #[serde(default)]
    pub model_override: Option<String>,
    /// Numero di layer di esperti MoE tenuti su CPU RAM (`--n-cpu-moe N`).
    /// None = non passato, ed è il default giusto: dalla 0.6.80 eullm calcola
    /// da sé quanti esperti spostare, leggendo i byte reali per tensore dalla
    /// sezione tensor-info del GGUF (non una stima su tipo e shape), e il
    /// modello si carica sempre — al peggio più lento, mai un OOM da
    /// dimensione.
    ///
    /// ATTENZIONE, è il motivo per cui questo campo NON va usato come tuning:
    /// l'auto-sizing di eullm si applica solo "when the user hasn't already
    /// chosen --cpu-moe/--n-cpu-moe themselves". Impostarlo qui DISATTIVA il
    /// calcolo automatico e inchioda il valore, quasi sempre peggio di quello
    /// che eullm ricaverebbe da solo.
    ///
    /// Resta utile per un caso solo: RISERVARE VRAM per qualcos'altro che al
    /// momento dell'avvio di eullm non è ancora allocato, e che quindi il suo
    /// probe (free_vram * 0.97 - 640 MiB) non può vedere. Per l'embedding
    /// residente non serve — lo copre l'ordine di avvio in main.rs, che carica
    /// bge-m3 prima di eullm proprio perché il probe lo veda.
    #[serde(default)]
    pub n_cpu_moe: Option<u32>,
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
    ///
    /// Con swap_during_ingestion=true smette di riguardare il caricamento
    /// iniziale (che parte sempre su CPU in quel caso) e governa invece lo
    /// swap verso GPU ad ogni ingestione: true = fa fallire quell'ingestione
    /// se lo swap non riesce, false = procede su CPU (più lento ma corretto).
    #[serde(default)]
    pub require_gpu: bool,
    /// Sposta bge-m3 dalla CPU (riposo) alla GPU SOLO durante la finestra di
    /// ingestione, poi torna in CPU — vedi AppState::swap_embeddings_to_gpu/
    /// _to_cpu e documents::upload(). Pensato per hardware dove bge-m3 e
    /// qwen non entrano insieme in VRAM (es. una scheda 12GB): fuori
    /// dall'ingestione la VRAM resta tutta a eullm, bge-m3 gira su CPU per
    /// l'unico embedding di ogni query (un testo corto, costo accettabile).
    /// Con VRAM abbondante (16GB+) non serve: lascialo a false, bge-m3
    /// resta sempre in GPU come oggi.
    ///
    /// Richiede EULLM__UNLOAD_DURING_INGESTION=true (senza l'unload di
    /// eullm non c'è VRAM libera in cui spostare bge-m3) e un binario
    /// compilato con --features cuda — Settings::load() fallisce all'avvio
    /// se una delle due condizioni manca, invece di degradare in silenzio.
    #[serde(default)]
    pub swap_during_ingestion: bool,
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
    fn default() -> Self {
        Self {
            model_id: default_embedding_model(),
            require_gpu: false,
            swap_during_ingestion: false,
        }
    }
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

        validate_swap_during_ingestion(
            s.embeddings.swap_during_ingestion,
            s.eullm.unload_during_ingestion,
            cfg!(feature = "cuda"),
        )?;

        Ok(s)
    }
}

/// Estratta da Settings::load per essere testabile senza toccare env var
/// reali (stesso motivo per cui ingestion_blocks in state.rs è una funzione
/// libera). Fallisce forte invece di ignorare in silenzio la combinazione
/// inconsistente — vedi EmbeddingsSettings::swap_during_ingestion.
fn validate_swap_during_ingestion(
    swap_during_ingestion: bool,
    unload_during_ingestion: bool,
    cuda_feature: bool,
) -> Result<()> {
    if !swap_during_ingestion {
        return Ok(());
    }
    if !unload_during_ingestion {
        anyhow::bail!(
            "EMBEDDINGS__SWAP_DURING_INGESTION=true richiede EULLM__UNLOAD_DURING_INGESTION=true \
             — bge-m3 si sposta in GPU solo nella VRAM che eullm libera con /api/unload, \
             senza quello non c'è spazio in cui spostarlo."
        );
    }
    if !cuda_feature {
        anyhow::bail!(
            "EMBEDDINGS__SWAP_DURING_INGESTION=true richiede un binario compilato con \
             --features cuda (lo swap verso GPU non è possibile senza supporto CUDA)."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_disabled_never_fails() {
        assert!(validate_swap_during_ingestion(false, false, false).is_ok());
        assert!(validate_swap_during_ingestion(false, true, true).is_ok());
    }

    #[test]
    fn swap_without_unload_fails() {
        let err = validate_swap_during_ingestion(true, false, true).unwrap_err();
        assert!(err.to_string().contains("UNLOAD_DURING_INGESTION"));
    }

    #[test]
    fn swap_without_cuda_feature_fails() {
        let err = validate_swap_during_ingestion(true, true, false).unwrap_err();
        assert!(err.to_string().contains("cuda"));
    }

    #[test]
    fn swap_with_unload_and_cuda_ok() {
        assert!(validate_swap_during_ingestion(true, true, true).is_ok());
    }
}
