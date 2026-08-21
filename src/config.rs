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
    #[serde(default)]
    pub eullm: EullmSettings,
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
    /// REST endpoint (HTTP/1.1) — used for the bootstrap healthcheck.
    #[serde(default = "default_qdrant_url")]
    pub url: String,
    /// gRPC endpoint (HTTP/2, tonic) — used by QdrantStore (qdrant-client).
    #[serde(default = "default_qdrant_grpc_url")]
    pub grpc_url: String,
    #[serde(default = "default_collection")]
    pub collection: String,
}

#[derive(Debug, Deserialize)]
pub struct EullmSettings {
    #[serde(default = "default_eullm_url")]
    pub url: String,
    /// Only used when manage_subprocesses=false (eullm managed externally):
    /// must be either a direct GGUF path or a name imported through
    /// `eullm import-ollama`. A bare name such as "qwen3:14b" with neither of
    /// those fails with a 500 "model not found". When the bootstrap starts
    /// eullm itself — the default — this value is ignored, and the same GGUF
    /// path passed to `eullm run` is used instead (see
    /// bootstrap::ProcessGuard::eullm_model_path and main.rs).
    ///
    /// It therefore has a default: requiring a setting that is ignored in the
    /// default configuration only stopped first runs for no reason.
    #[serde(default = "default_eullm_model")]
    pub model: String,
    /// Context PER CONNECTION (slot). The --ctx-size flag passed to eullm at
    /// startup is the TOTAL: num_ctx * batch_size (see bootstrap::spawn_eullm).
    #[serde(default = "default_num_ctx")]
    pub num_ctx: u32,
    #[serde(default = "default_num_predict")]
    pub num_predict: u32,
    #[serde(default = "default_repeat_penalty")]
    pub repeat_penalty: f32,
    /// How many recent tokens repeat_penalty looks back over (llama.cpp
    /// `repeat_last_n`). Was hardcoded to 256 with no way to override it —
    /// Ollama's own default is 64 (repeat_penalty 1.1, vs our 1.3): a
    /// smaller window and a lighter penalty. Default here stays 256 to keep
    /// existing deployments byte-for-byte unchanged; lower it (e.g. 64) to
    /// match Ollama's proven-working combination if generation quality
    /// regresses relative to the python-legacy stack.
    #[serde(default = "default_repeat_last_n")]
    pub repeat_last_n: u32,
    #[serde(default = "default_keep_alive")]
    pub keep_alive: i32,
    /// Concurrent connections handled by eullm. KV cache VRAM scales linearly
    /// with num_ctx * batch_size — check the VRAM budget before raising it
    /// (see BUILD.md).
    #[serde(default = "default_eullm_batch_size")]
    pub batch_size: u32,
    /// KV cache quantisation (llama.cpp: f16, q8_0, q4_0…). None = eullm's own
    /// default (F16, no flag passed). Test it per platform and GPU: do not
    /// assume it is honoured — check eullm's logs.
    #[serde(default)]
    pub cache_type_k: Option<String>,
    #[serde(default)]
    pub cache_type_v: Option<String>,
    // NOTE: the `fit` field was removed together with the 0.6.80 pin. From that
    // version on, eullm sizes the GPU offload by itself, ALWAYS, regardless of
    // the flag: passing `--fit` explicitly only asks for confirmation on a
    // partial split, and only when stdin AND stdout are both TTYs
    // (fit.rs:851 in eullm's source) — we spawn with stdin on null, so for us
    // it was a no-op. The startup order the flag used to govern is now
    // unconditional in main.rs. Existing configs carrying `fit = ...` still
    // load: the struct does not use deny_unknown_fields, so it is ignored.
    /// Evict eullm from VRAM for the duration of document ingestion
    /// (POST /api/unload — an EULLM extension verified in eullm's source:
    /// api_routes() registers `.route("/unload", post(unload_model))`, mounted
    /// under /api). While this is active chat does not work, because eullm is
    /// not resident: the UI shows "ingestion in progress" but does not block
    /// sending, so a question asked in that window either fails or waits until
    /// the reload.
    #[serde(default)]
    pub unload_during_ingestion: bool,
    /// Overrides which model to start, ONLY when manage_subprocesses=true
    /// (otherwise `model` above is used as-is). When Some, it bypasses the
    /// lookup of the "qwen3-14b" component pinned in the manifest and passes
    /// this value straight to `eullm run` — either a local GGUF path or an
    /// `hf.co/user/repo:quant` reference that eullm resolves and downloads on
    /// its own, outside our sha256-pinned manifest. Integrity verification is
    /// then eullm's and the HF hub's responsibility, not ours.
    #[serde(default)]
    pub model_override: Option<String>,
    /// Number of MoE expert layers kept in CPU RAM (`--n-cpu-moe N`).
    /// None = not passed, and that is the right default: from 0.6.80 eullm
    /// works out how many experts to move by itself, reading the real
    /// per-tensor byte sizes from the GGUF's tensor-info section rather than
    /// estimating from type and shape. The model always loads — at worst more
    /// slowly, never with a size-related OOM.
    ///
    /// CAREFUL — this is exactly why this field must NOT be used as a tuning
    /// knob: eullm's auto-sizing only applies "when the user hasn't already
    /// chosen --cpu-moe/--n-cpu-moe themselves". Setting it here DISABLES the
    /// automatic computation and nails the value down, almost always worse
    /// than what eullm would derive on its own.
    ///
    /// It stays useful for one case only: RESERVING VRAM for something that is
    /// not yet allocated when eullm starts, and which its probe
    /// (free_vram * 0.97 - 640 MiB) therefore cannot see. It is not needed for
    /// the resident embedding model — the startup order in main.rs covers that
    /// by loading bge-m3 before eullm, precisely so the probe sees it.
    #[serde(default)]
    pub n_cpu_moe: Option<u32>,
    /// Only consulted when embeddings.ingestion_embedding=Eullm — see its doc
    /// comment. Governs whether bootstrap::spawn_eullm ALSO passes
    /// --embedding-model (eullm >= 0.6.90) at startup, permanently reserving
    /// bge-m3 a place in VRAM next to the chat model, or leaves it out,
    /// relying purely on POST /api/embed's own on-demand coexist-or-evict
    /// (eullm >= 0.6.82) each time an ingestion actually needs it.
    ///
    /// These are NOT two ways to reach the same outcome — they suit opposite
    /// hardware, and picking the wrong one is worse than picking neither:
    ///
    ///   - true: for a card known to comfortably fit both models together.
    ///     Zero swap overhead, ever — bge-m3 loads once, at eullm's own
    ///     startup, and stays. --embedding-model's reservation runs BEFORE
    ///     --fit sizes the chat model, so on a card that does NOT fit both,
    ///     it is bge-m3 that claims VRAM first and the chat model that gets
    ///     squeezed — the opposite of what a RAG deployment wants.
    ///   - false (default): for a card too small, or of unknown size, to fit
    ///     both permanently. eullm measures free VRAM on every /api/embed
    ///     call and decides then whether bge-m3 fits alongside the chat model
    ///     or must evict it, reloading the chat model automatically on the
    ///     next /api/generate — the same unload/reload dance this project
    ///     used to orchestrate by hand (POST /api/unload before, reload()
    ///     after — see documents::upload), now internal to eullm and
    ///     triggered by the ordinary act of calling it, no special
    ///     coordination required on this side.
    #[serde(default)]
    pub reserve_embedding_model: bool,
}

/// How bge-m3 embedding happens — always bge-m3, but through a different
/// device or process depending on this choice. Off and CandleGpu affect
/// ONLY the ingestion window (documents::upload); query-time embedding
/// (api/query.rs::prepare, a single short text per request) always goes
/// through the resident Candle instance for those two. Eullm is the
/// exception: it routes BOTH ingestion and query embedding through eullm,
/// and Candle is not loaded at all in that mode — see the Eullm variant.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IngestionEmbedding {
    /// bge-m3 (Candle) does not change device for ingestion — the
    /// historical default. require_gpu governs the one, permanent, initial
    /// load as usual.
    #[default]
    Off,
    /// bge-m3 (Candle) moves from CPU to GPU ONLY for the ingestion window,
    /// then back — see AppState::swap_embeddings_to_gpu/_to_cpu and
    /// documents::upload(). Meant for hardware where bge-m3 and qwen do not
    /// fit in VRAM together, e.g. a 12GB card: outside ingestion all the
    /// VRAM stays with eullm, and bge-m3 runs on CPU for the single
    /// embedding each query needs. With plenty of VRAM (16GB+) this is
    /// unnecessary — leave it Off and bge-m3 stays resident on the GPU.
    ///
    /// Requires EULLM__UNLOAD_DURING_INGESTION=true — without evicting
    /// eullm there is no free VRAM to move bge-m3 into — and a binary built
    /// with --features cuda. Settings::load() fails at startup if either
    /// condition is missing, rather than degrading silently.
    CandleGpu,
    /// Candle is not loaded at startup at all — not even on CPU (see
    /// main::load_embedding) — and bootstrap does not download its ~2.1GB
    /// of bge-m3 weights either (see bootstrap::drop_unused_embedding_model).
    /// BOTH ingestion (api/documents.rs) and query embedding
    /// (api/query.rs::prepare) go through eullm's own "bge-m3" store entry
    /// instead (see manifest.toml, and EULLM_MODELS_DIR in
    /// bootstrap::spawn_eullm), over POST /api/embed. eullm decides on its
    /// own whether to evict the chat model to make room, the same VRAM
    /// management it already does unprompted — no manual /api/unload from
    /// this process. Needs no --features cuda: this binary never touches
    /// CUDA itself in this mode, eullm does.
    ///
    /// Two consequences worth knowing before enabling this: (1) chat queries
    /// are blocked for the whole ingestion window regardless of whether
    /// eullm actually evicts anything — see AppState::ingestion_blocks_queries
    /// — and (2) on a card where bge-m3 and the chat model do not both fit,
    /// EVERY query now pays a potential double swap (evict chat to embed the
    /// question, evict bge-m3 back out to answer it) — harmless when both
    /// fit together, real added latency per query when they do not.
    Eullm,
}

/// eullm's own name, in ITS store (EULLM_MODELS_DIR/bge-m3/*.gguf — see
/// manifest.toml's bge-m3-gguf component and bootstrap::spawn_eullm), for
/// the model IngestionEmbedding::Eullm asks it to embed with. Not a free
/// filesystem path: eullm resolves "model" in a request against a store
/// name, exactly like it already does for the chat model — a path eullm
/// was not started with answers 404 regardless of /api/unload, confirmed
/// against a real eullm 0.6.82 server, not assumed.
pub const EULLM_EMBEDDING_MODEL: &str = "bge-m3";

#[derive(Debug, Deserialize)]
pub struct EmbeddingsSettings {
    #[serde(default = "default_embedding_model")]
    pub model_id: String,
    /// When true, CUDA is mandatory for embeddings. If CUDA init fails after
    /// the retries (see EmbeddingService::load) startup FAILS rather than
    /// silently degrading to CPU — an ingestion taking 17 minutes instead of
    /// seconds must never go unnoticed. Default false: the CPU fallback is
    /// allowed, but logged at error level (not warn) and exposed via GET /info.
    /// Env: EMBEDDINGS__REQUIRE_GPU (the Settings field is named "embeddings").
    ///
    /// With ingestion_embedding=CandleGpu it stops governing the initial
    /// load, which always starts on CPU in that mode, and governs the swap
    /// to GPU on each ingestion instead: true = fail that ingestion if the
    /// swap does not succeed, false = carry on using CPU (slower but
    /// correct). With ingestion_embedding=Eullm this binary never asks
    /// Candle for CUDA at all, so require_gpu is free to be false, and
    /// should be — the GPU belongs to eullm in that mode.
    #[serde(default)]
    pub require_gpu: bool,
    /// See IngestionEmbedding. Env: EMBEDDINGS__INGESTION_EMBEDDING
    /// ("off" | "candle_gpu" | "eullm").
    #[serde(default)]
    pub ingestion_embedding: IngestionEmbedding,
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
    /// Document upload limit (MB). MAX_UPLOAD_SIZE_MB, default 100.
    #[serde(default = "default_max_upload_mb")]
    pub max_upload_mb: u64,
}

/// Radice dati: binari, modelli, storage Qdrant, db SQLite, uploads.
/// Layout: {dir}/bin/  {dir}/models/  {dir}/storage/  {dir}/db/  {dir}/uploads/
#[derive(Debug, Deserialize)]
pub struct DataSettings {
    /// Percorso radice (default: cartella dell'eseguibile — portable app dir).
    /// Dev override: DATA__DIR=/path/to/working/dir (the binary lives in target/debug/).
    #[serde(default = "default_data_dir")]
    pub dir: String,
    /// When true, the bootstrap starts and supervises qdrant and eullm as child
    /// processes. When false, it expects them to be listening already (dev/compose).
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
impl Default for EullmSettings {
    fn default() -> Self {
        Self {
            url: default_eullm_url(),
            model: default_eullm_model(),
            num_ctx: default_num_ctx(),
            num_predict: default_num_predict(),
            repeat_penalty: default_repeat_penalty(),
            repeat_last_n: default_repeat_last_n(),
            keep_alive: default_keep_alive(),
            batch_size: default_eullm_batch_size(),
            cache_type_k: None,
            cache_type_v: None,
            unload_during_ingestion: false,
            model_override: None,
            n_cpu_moe: None,
            reserve_embedding_model: false,
        }
    }
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
            ingestion_embedding: IngestionEmbedding::default(),
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
fn default_eullm_model() -> String { "qwen3-14b".into() }
fn default_num_ctx() -> u32 { 16384 }
fn default_eullm_batch_size() -> u32 { 1 }
fn default_num_predict() -> u32 { 4096 }
fn default_repeat_penalty() -> f32 { 1.3 }
fn default_repeat_last_n() -> u32 { 256 }
fn default_keep_alive() -> i32 { -1 }
fn default_embedding_model() -> String { "BAAI/bge-m3".into() }
fn default_backup_dir() -> String { "./backups".into() }
fn default_documents_dir() -> String { "./documents".into() }
fn default_max_upload_mb() -> u64 { 100 }
fn default_data_dir() -> String {
    // Exe-relative: the directory holding the binary is the portable app dir.
    // In production: /install/dir/i3k-rag-engine → data = /install/dir/
    // In dev (cargo run): target/debug/ → override with DATA__DIR=./data
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
        // Look for .env in the CWD first (dev: cargo run from the repo root),
        // then next to the executable (production: binary dir).
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

        // Derive paths from data_dir when they are still at their defaults, so
        // that setting DATA__DIR alone relocates everything.
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

        validate_ingestion_embedding(
            s.embeddings.ingestion_embedding,
            s.eullm.unload_during_ingestion,
            s.eullm.reserve_embedding_model,
            cfg!(feature = "cuda"),
        )?;

        Ok(s)
    }
}

/// Extracted from Settings::load so it can be tested without touching real
/// environment variables — the same reason ingestion_blocks in state.rs is a
/// free function. Fails loudly instead of silently ignoring an inconsistent
/// combination; see IngestionEmbedding.
fn validate_ingestion_embedding(
    ingestion_embedding: IngestionEmbedding,
    unload_during_ingestion: bool,
    reserve_embedding_model: bool,
    cuda_feature: bool,
) -> Result<()> {
    match ingestion_embedding {
        IngestionEmbedding::Off | IngestionEmbedding::CandleGpu
            if reserve_embedding_model =>
        {
            anyhow::bail!(
                "EULLM__RESERVE_EMBEDDING_MODEL=true has no effect without \
                 EMBEDDINGS__INGESTION_EMBEDDING=eullm — it governs whether eullm is started \
                 with --embedding-model, which only matters when this process actually asks \
                 eullm to embed with it."
            );
        }
        IngestionEmbedding::Off => Ok(()),
        IngestionEmbedding::CandleGpu => {
            if !unload_during_ingestion {
                anyhow::bail!(
                    "EMBEDDINGS__INGESTION_EMBEDDING=candle_gpu requires \
                     EULLM__UNLOAD_DURING_INGESTION=true — bge-m3 can only move into the VRAM \
                     that eullm releases through /api/unload, and without that there is \
                     nowhere to move it."
                );
            }
            if !cuda_feature {
                anyhow::bail!(
                    "EMBEDDINGS__INGESTION_EMBEDDING=candle_gpu requires a binary built with \
                     --features cuda (swapping to GPU is impossible without CUDA support)."
                );
            }
            Ok(())
        }
        IngestionEmbedding::Eullm => {
            // No requirement on unload_during_ingestion or the cuda feature:
            // eullm owns both the GPU and the decision to evict its own chat
            // model, unprompted — see IngestionEmbedding::Eullm's doc comment.
            // unload_during_ingestion=true together with this IS allowed (it
            // just adds a redundant, harmless manual unload before eullm's
            // own automatic one), not worth rejecting. reserve_embedding_model
            // is a free choice either way — see its own doc comment for which
            // hardware wants which value; wrong-for-your-card is a real
            // footgun (--embedding-model can starve the chat model on a card
            // too small for both) but not an invalid *combination* the way
            // the Off/CandleGpu case above is.
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingestion_embedding_off_never_fails() {
        assert!(validate_ingestion_embedding(IngestionEmbedding::Off, false, false, false).is_ok());
        assert!(validate_ingestion_embedding(IngestionEmbedding::Off, true, false, true).is_ok());
    }

    #[test]
    fn candle_gpu_without_unload_fails() {
        let err = validate_ingestion_embedding(IngestionEmbedding::CandleGpu, false, false, true)
            .unwrap_err();
        assert!(err.to_string().contains("UNLOAD_DURING_INGESTION"));
    }

    #[test]
    fn candle_gpu_without_cuda_feature_fails() {
        let err = validate_ingestion_embedding(IngestionEmbedding::CandleGpu, true, false, false)
            .unwrap_err();
        assert!(err.to_string().contains("cuda"));
    }

    #[test]
    fn candle_gpu_with_unload_and_cuda_ok() {
        assert!(
            validate_ingestion_embedding(IngestionEmbedding::CandleGpu, true, false, true).is_ok()
        );
    }

    #[test]
    fn eullm_never_fails_regardless_of_unload_reserve_or_cuda() {
        for unload in [false, true] {
            for reserve in [false, true] {
                for cuda in [false, true] {
                    assert!(
                        validate_ingestion_embedding(IngestionEmbedding::Eullm, unload, reserve, cuda)
                            .is_ok(),
                        "unload={unload} reserve={reserve} cuda={cuda}"
                    );
                }
            }
        }
    }

    /// The new rejection case: reserve_embedding_model only means anything
    /// when this process actually asks eullm to embed (IngestionEmbedding::
    /// Eullm) — with Off or CandleGpu, bge-m3 either never runs through
    /// eullm or is Candle's concern, so a --embedding-model reservation
    /// would sit there unused. Caught at startup rather than silently
    /// ignored, same as every other invalid combination in this function.
    #[test]
    fn reserve_embedding_model_without_eullm_fails() {
        let err =
            validate_ingestion_embedding(IngestionEmbedding::Off, false, true, false).unwrap_err();
        assert!(err.to_string().contains("RESERVE_EMBEDDING_MODEL"));

        let err = validate_ingestion_embedding(IngestionEmbedding::CandleGpu, true, true, true)
            .unwrap_err();
        assert!(err.to_string().contains("RESERVE_EMBEDDING_MODEL"));
    }
}
