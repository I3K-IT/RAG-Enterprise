//! --bench/--benchmark: misura ingestione+inferenza su un documento reale,
//! con l'hardware effettivamente in uso, e scrive un report Markdown (con
//! grafici a torta Mermaid) pensato per individuare colli di bottiglia.
//!
//! Riusa lo stesso bootstrap (qdrant/eullm/embedding) dell'avvio normale —
//! niente scorciatoie: i tempi devono riflettere condizioni reali. Scrive in
//! una collection Qdrant dedicata ("{collection}_benchmark", azzerata ad ogni
//! run) per non toccare mai i dati reali dell'utente.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use qdrant_client::Qdrant;

use crate::clients::embeddings::EmbeddingService;
use crate::clients::eullm::EullmClient;
use crate::clients::qdrant_store::QdrantStore;
use crate::config::Settings;
use crate::documents::parser;
use crate::rag::{chunker, prompt, retrieval, vector_store::ChunkPayload, vector_store::VectorStore};

// ── CLI ──────────────────────────────────────────────────────────────────────

pub struct BenchArgs {
    pub doc_path: PathBuf,
    pub queries: Vec<String>,
}

/// `--bench <path>` o `--benchmark <path>`, con zero o più `--bench-query "…"`
/// ripetuti. Se nessuna query è passata, run() usa un piccolo set generico.
pub fn parse_args(args: &[String]) -> Option<BenchArgs> {
    let bench_idx = args.iter().position(|a| a == "--bench" || a == "--benchmark")?;
    let doc_path = PathBuf::from(args.get(bench_idx + 1)?);

    let mut queries = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--bench-query" {
            queries.push(args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    Some(BenchArgs { doc_path, queries })
}

/// `--bench-live`: modalità alternativa a `--bench <file>` — il server e il
/// frontend partono normalmente, e OGNI ingestione/query reale fatta durante
/// la sessione viene cronometrata e registrata (vedi LiveRecorder). Il
/// report viene scritto alla chiusura (SIGINT/SIGTERM, vedi main.rs). A
/// differenza di `--bench <file>` il carico non è fisso: utile per capire il
/// comportamento sotto uso reale, non per confrontare hardware diversi a
/// parità di carico (per quello serve `--bench <file>`, riproducibile).
pub fn live_mode_requested(args: &[String]) -> bool {
    args.iter().any(|a| a == "--bench-live")
}

fn default_queries() -> Vec<String> {
    vec![
        "Riassumi in breve il contenuto di questo documento.".to_owned(),
        "Quali sono i punti principali trattati nel documento?".to_owned(),
    ]
}

// ── Hardware ─────────────────────────────────────────────────────────────────

pub struct HardwareInfo {
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub ram_total_mb: u64,
    pub gpu_name: Option<String>,
    pub gpu_vram_total_mb: Option<u64>,
    pub gpu_vram_free_mb: Option<u64>,
    pub os: String,
    pub embedding_device: String,
    pub eullm_model: String,
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_else(|| "sconosciuta".to_owned())
}

fn ram_total_mb() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

fn os_info() -> String {
    let pretty_name = std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("PRETTY_NAME="))
            .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_owned())
    });
    let kernel = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    match (pretty_name, kernel) {
        (Some(p), Some(k)) => format!("{p} (kernel {k})"),
        (Some(p), None) => p,
        (None, Some(k)) => format!("Linux (kernel {k})"),
        (None, None) => format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    }
}

/// nvidia-smi, non le API CUDA dirette: funziona indipendentemente da come è
/// stato compilato questo binario (--features cuda o no), e riflette lo stato
/// REALE della GPU al momento del benchmark (VRAM libera cambia in continuazione).
fn gpu_info() -> (Option<String>, Option<u64>, Option<u64>) {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total,memory.free", "--format=csv,noheader,nounits"])
        .output();
    let Ok(output) = output else {
        return (None, None, None);
    };
    if !output.status.success() {
        return (None, None, None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split(',').map(|s| s.trim()).collect();
    match parts.as_slice() {
        [name, total, free] => (
            Some(name.to_string()),
            total.parse::<u64>().ok(),
            free.parse::<u64>().ok(),
        ),
        _ => (None, None, None),
    }
}

pub fn collect_hardware_info(embeddings: &EmbeddingService, eullm_model: &str) -> HardwareInfo {
    let (gpu_name, gpu_vram_total_mb, gpu_vram_free_mb) = gpu_info();
    HardwareInfo {
        cpu_model: cpu_model(),
        cpu_cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        ram_total_mb: ram_total_mb(),
        gpu_name,
        gpu_vram_total_mb,
        gpu_vram_free_mb,
        os: os_info(),
        embedding_device: embeddings.device_label().to_owned(),
        eullm_model: eullm_model.to_owned(),
    }
}

// ── Timing ───────────────────────────────────────────────────────────────────

pub struct StageTiming {
    pub name: &'static str,
    pub duration: Duration,
}

impl StageTiming {
    fn ms(&self) -> f64 {
        self.duration.as_secs_f64() * 1000.0
    }
}

pub struct IngestionResult {
    pub document_id: String,
    pub stages: Vec<StageTiming>,
    pub page_count: Option<u32>,
    pub word_count: usize,
    pub char_count: usize,
    pub chunk_count: usize,
}

impl IngestionResult {
    fn total(&self) -> Duration {
        self.stages.iter().map(|s| s.duration).sum()
    }
}

pub struct InferenceResult {
    pub query: String,
    pub embed_query: Duration,
    pub search: Duration,
    pub prompt_build: Duration,
    pub ttft: Duration,
    pub total_generation: Duration,
    pub tokens_generated: usize,
    pub chunks_retrieved: usize,
    pub chunks_from_bench_doc: usize,
}

impl InferenceResult {
    /// token/sec nella sola fase di decode (esclude il prefill/TTFT) — è la
    /// velocità "di regime", quella che conta per risposte lunghe. Con un
    /// solo token generato non è definita (nessun intervallo decode misurabile).
    pub fn decode_tokens_per_sec(&self) -> Option<f64> {
        if self.tokens_generated < 2 {
            return None;
        }
        let decode_time = self.total_generation.saturating_sub(self.ttft).as_secs_f64();
        if decode_time <= 0.0 {
            return None;
        }
        Some((self.tokens_generated - 1) as f64 / decode_time)
    }
}

// ── Ingestione ───────────────────────────────────────────────────────────────

async fn run_ingestion(
    doc_path: &Path,
    settings: &Settings,
    embeddings: &EmbeddingService,
    qdrant: &QdrantStore,
) -> Result<IngestionResult> {
    let data_dir = settings.data.data_path();
    let doc_path_owned = doc_path.to_path_buf();

    let t = Instant::now();
    let (text, page_count) = tokio::task::spawn_blocking(move || parser::extract_text(&doc_path_owned, &data_dir))
        .await
        .context("join estrazione testo")?
        .with_context(|| format!("estrazione testo da {}", doc_path.display()))?;
    let extract_time = t.elapsed();
    tracing::info!(
        pages = ?page_count, chars = text.chars().count(), ms = extract_time.as_millis(),
        "estrazione completata"
    );

    let word_count = text.split_whitespace().count();
    let char_count = text.chars().count();

    let t = Instant::now();
    let chunks = chunker::split_text(&text);
    let chunk_time = t.elapsed();
    if chunks.is_empty() {
        anyhow::bail!("nessun testo estratto da {} — impossibile fare benchmark", doc_path.display());
    }
    // Silenzio radio tra qui e la fine dell'embedding può durare minuti sui
    // documenti grandi (centinaia di chunk, un'unica chiamata embed_texts) —
    // questo log evita che sembri bloccato quando sta solo lavorando.
    tracing::info!(
        chunks = chunks.len(), ms = chunk_time.as_millis(),
        "chunking completato, avvio embedding (può richiedere qualche minuto sui documenti grandi)"
    );

    let t = Instant::now();
    let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
    let embedding_vecs = embeddings.embed_texts(&chunk_refs).context("embedding chunk")?;
    let embed_time = t.elapsed();
    tracing::info!(ms = embed_time.as_millis(), "embedding completato, avvio upsert su Qdrant");

    let document_id = uuid::Uuid::new_v4().to_string();
    let filename = doc_path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = doc_path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let upload_date = chrono::Utc::now().to_rfc3339();
    let payloads: Vec<ChunkPayload> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| ChunkPayload {
            document_id: document_id.clone(),
            chunk_index: i,
            filename: filename.clone(),
            upload_date: upload_date.clone(),
            text: c.clone(),
            chunk_size: c.len(),
            document_type: ext.clone(),
            structured_fields: None,
        })
        .collect();

    let t = Instant::now();
    qdrant.upsert(&embedding_vecs, &payloads).await.context("upsert Qdrant")?;
    let upsert_time = t.elapsed();
    tracing::info!(ms = upsert_time.as_millis(), "upsert completato, ingestione finita");

    Ok(IngestionResult {
        document_id,
        stages: vec![
            StageTiming { name: "Estrazione testo", duration: extract_time },
            StageTiming { name: "Chunking", duration: chunk_time },
            StageTiming { name: "Embedding", duration: embed_time },
            StageTiming { name: "Upsert Qdrant", duration: upsert_time },
        ],
        page_count,
        word_count,
        char_count,
        chunk_count: chunks.len(),
    })
}

// ── Inferenza ────────────────────────────────────────────────────────────────

async fn run_inference(
    query: &str,
    bench_document_id: &str,
    embeddings: &EmbeddingService,
    qdrant: &QdrantStore,
    eullm: &std::sync::Arc<EullmClient>,
) -> Result<InferenceResult> {
    let t = Instant::now();
    let query_vec = embeddings.embed_text(query).context("embedding query")?;
    let embed_query = t.elapsed();

    let t = Instant::now();
    let hits = qdrant
        .search(query_vec, retrieval::TOP_K, Some(retrieval::RELEVANCE_THRESHOLD))
        .await
        .context("ricerca Qdrant")?;
    let search = t.elapsed();
    tracing::info!(chunks_retrieved = hits.len(), ms = search.as_millis(), "ricerca completata");

    let chunks_from_bench_doc = hits.iter().filter(|h| h.payload.document_id == bench_document_id).count();

    let context: String = hits
        .iter()
        .map(|h| format!("[{}]\n{}", h.payload.filename, h.payload.text))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let t = Instant::now();
    let full_prompt = prompt::build_prompt(&context, query, &[]);
    let prompt_build = t.elapsed();

    tracing::info!("avvio generazione eullm");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let eullm_task = eullm.clone();
    let prompt_for_task = full_prompt.clone();
    let gen_handle = tokio::spawn(async move { eullm_task.invoke_stream(&prompt_for_task, tx).await });

    let gen_start = Instant::now();
    let mut ttft: Option<Duration> = None;
    let mut tokens_generated = 0usize;
    while let Some(_token) = rx.recv().await {
        if ttft.is_none() {
            ttft = Some(gen_start.elapsed());
            tracing::info!(ttft_ms = gen_start.elapsed().as_millis(), "primo token ricevuto");
        }
        tokens_generated += 1;
    }
    let total_generation = gen_start.elapsed();
    gen_handle.await.context("join generazione eullm")?.context("eullm invoke_stream")?;
    tracing::info!(
        tokens = tokens_generated, ms = total_generation.as_millis(),
        "generazione completata"
    );

    Ok(InferenceResult {
        query: query.to_owned(),
        embed_query,
        search,
        prompt_build,
        ttft: ttft.unwrap_or(total_generation),
        total_generation,
        tokens_generated,
        chunks_retrieved: hits.len(),
        chunks_from_bench_doc,
    })
}

// ── Collection Qdrant dedicata ──────────────────────────────────────────────

/// Azzera la collection di benchmark prima di ogni run, così i risultati non
/// si mescolano mai con quelli di run precedenti — mai la collection reale
/// dell'utente (QdrantStore::new la ricrea da zero se non esiste, vedi
/// ensure_collection in qdrant_store.rs).
async fn reset_benchmark_collection(grpc_url: &str, collection: &str) -> Result<()> {
    let client = Qdrant::from_url(grpc_url).build().context("connessione Qdrant")?;
    if client.collection_exists(collection).await.context("collection_exists")? {
        client.delete_collection(collection).await.context("delete_collection")?;
    }
    Ok(())
}

// ── Report ───────────────────────────────────────────────────────────────────

fn print_summary(hw: &HardwareInfo, ingestion: &IngestionResult, inferences: &[InferenceResult]) {
    println!();
    println!("=== Benchmark i3k-rag-engine ===");
    println!("CPU: {} ({} core) | RAM: {} MB", hw.cpu_model, hw.cpu_cores, hw.ram_total_mb);
    match (&hw.gpu_name, hw.gpu_vram_free_mb, hw.gpu_vram_total_mb) {
        (Some(name), Some(free), Some(total)) => {
            println!("GPU: {name} | VRAM libera: {free}/{total} MB")
        }
        _ => println!("GPU: non rilevata (nvidia-smi non disponibile o nessuna GPU)"),
    }
    println!("Embedding device: {}", hw.embedding_device);
    println!();
    println!(
        "Ingestione — {} pagine, {} parole, {} chunk — totale {:.0} ms",
        hw_opt(ingestion.page_count),
        ingestion.word_count,
        ingestion.chunk_count,
        ingestion.total().as_secs_f64() * 1000.0
    );
    for s in &ingestion.stages {
        println!("  {:<20} {:>8.0} ms", s.name, s.ms());
    }
    println!();
    for (i, inf) in inferences.iter().enumerate() {
        println!("Inferenza #{} — \"{}\"", i + 1, truncate(&inf.query, 60));
        println!(
            "  chunk recuperati: {} (di cui {} dal documento benchmark)",
            inf.chunks_retrieved, inf.chunks_from_bench_doc
        );
        println!("  TTFT: {:.0} ms | generazione totale: {:.0} ms | token: {}",
            inf.ttft.as_secs_f64() * 1000.0,
            inf.total_generation.as_secs_f64() * 1000.0,
            inf.tokens_generated
        );
        if let Some(tps) = inf.decode_tokens_per_sec() {
            println!("  velocità decode: {tps:.1} token/sec");
        }
    }
    println!();
}

fn hw_opt(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "N/A".to_owned())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn write_markdown_report(
    hw: &HardwareInfo,
    doc_path: &Path,
    ingestion: &IngestionResult,
    inferences: &[InferenceResult],
) -> Result<PathBuf> {
    let now = chrono::Local::now();
    let path = PathBuf::from(format!("benchmark-report-{}.md", now.format("%Y%m%d-%H%M%S")));

    let mut md = String::new();
    md.push_str(&format!("# Benchmark i3k-rag-engine — {}\n\n", now.format("%Y-%m-%d %H:%M:%S")));

    md.push_str("## Hardware\n\n");
    md.push_str("| Componente | Dettaglio |\n|---|---|\n");
    md.push_str(&format!("| CPU | {} ({} core) |\n", hw.cpu_model, hw.cpu_cores));
    md.push_str(&format!("| RAM | {} MB |\n", hw.ram_total_mb));
    match (&hw.gpu_name, hw.gpu_vram_total_mb, hw.gpu_vram_free_mb) {
        (Some(name), Some(total), Some(free)) => {
            md.push_str(&format!("| GPU | {name} |\n"));
            md.push_str(&format!("| VRAM | {free} MB liberi / {total} MB totali |\n"));
        }
        _ => md.push_str("| GPU | non rilevata (nvidia-smi non disponibile o nessuna GPU) |\n"),
    }
    md.push_str(&format!("| OS | {} |\n", hw.os));
    md.push_str(&format!("| Device embedding | {} |\n", hw.embedding_device));
    md.push_str(&format!("| Modello eullm | {} |\n", hw.eullm_model));

    md.push_str("\n## Documento\n\n");
    md.push_str(&format!("- File: `{}`\n", doc_path.display()));
    md.push_str(&format!("- Pagine: {}\n", hw_opt(ingestion.page_count)));
    md.push_str(&format!("- Parole: {}\n", ingestion.word_count));
    md.push_str(&format!("- Caratteri: {}\n", ingestion.char_count));
    md.push_str(&format!("- Chunk generati: {}\n", ingestion.chunk_count));

    md.push_str("\n## Ingestione — tempi per fase\n\n");
    md.push_str("| Fase | Tempo (ms) | % del totale |\n|---|---|---|\n");
    let ingestion_total_ms = (ingestion.total().as_secs_f64() * 1000.0).max(0.001);
    for s in &ingestion.stages {
        md.push_str(&format!("| {} | {:.0} | {:.1}% |\n", s.name, s.ms(), s.ms() / ingestion_total_ms * 100.0));
    }
    md.push_str(&format!("| **Totale** | **{:.0}** | **100%** |\n", ingestion_total_ms));

    md.push_str("\n```mermaid\npie title Tempo di ingestione per fase\n");
    for s in &ingestion.stages {
        md.push_str(&format!("    \"{}\" : {:.1}\n", s.name, s.ms().max(0.1)));
    }
    md.push_str("```\n");

    if let Some(worst) = ingestion.stages.iter().max_by(|a, b| a.duration.cmp(&b.duration)) {
        md.push_str(&format!(
            "\n**Collo di bottiglia ingestione**: {} ({:.1}% del tempo totale)\n",
            worst.name,
            worst.ms() / ingestion_total_ms * 100.0
        ));
    }

    md.push_str("\n## Inferenza\n\n");
    for (i, inf) in inferences.iter().enumerate() {
        md.push_str(&format!("### Query {}: \"{}\"\n\n", i + 1, inf.query));
        md.push_str("| Fase | Tempo (ms) |\n|---|---|\n");
        md.push_str(&format!("| Embedding query | {:.0} |\n", inf.embed_query.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Ricerca Qdrant | {:.0} |\n", inf.search.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Costruzione prompt | {:.0} |\n", inf.prompt_build.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Tempo alla prima parola (TTFT / prefill) | {:.0} |\n", inf.ttft.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Generazione totale | {:.0} |\n", inf.total_generation.as_secs_f64() * 1000.0));

        md.push_str(&format!(
            "\n- Chunk recuperati: {} (top_k={}, soglia similarità ≥ {}), di cui {} dal documento appena ingerito\n",
            inf.chunks_retrieved, retrieval::TOP_K, retrieval::RELEVANCE_THRESHOLD, inf.chunks_from_bench_doc
        ));
        md.push_str(&format!("- Token generati: {}\n", inf.tokens_generated));
        match inf.decode_tokens_per_sec() {
            Some(tps) => md.push_str(&format!("- Velocità decode (esclude prefill): {tps:.1} token/sec\n")),
            None => md.push_str("- Velocità decode: N/A (troppo pochi token generati)\n"),
        }

        let decode_ms = inf.total_generation.saturating_sub(inf.ttft).as_secs_f64() * 1000.0;
        md.push_str("\n```mermaid\npie title Tempo di inferenza per fase\n");
        md.push_str(&format!("    \"Embedding query\" : {:.1}\n", (inf.embed_query.as_secs_f64() * 1000.0).max(0.1)));
        md.push_str(&format!("    \"Ricerca Qdrant\" : {:.1}\n", (inf.search.as_secs_f64() * 1000.0).max(0.1)));
        md.push_str(&format!("    \"Prefill (TTFT)\" : {:.1}\n", (inf.ttft.as_secs_f64() * 1000.0).max(0.1)));
        md.push_str(&format!("    \"Decode\" : {:.1}\n", decode_ms.max(0.1)));
        md.push_str("```\n\n");
    }

    md.push_str("## Riepilogo colli di bottiglia\n\n");
    let mut all_stages: Vec<(String, f64)> = ingestion
        .stages
        .iter()
        .map(|s| (format!("Ingestione: {}", s.name), s.ms()))
        .collect();
    for (i, inf) in inferences.iter().enumerate() {
        all_stages.push((format!("Query {}: embedding", i + 1), inf.embed_query.as_secs_f64() * 1000.0));
        all_stages.push((format!("Query {}: ricerca Qdrant", i + 1), inf.search.as_secs_f64() * 1000.0));
        all_stages.push((format!("Query {}: prefill/TTFT", i + 1), inf.ttft.as_secs_f64() * 1000.0));
        all_stages.push((
            format!("Query {}: decode", i + 1),
            inf.total_generation.saturating_sub(inf.ttft).as_secs_f64() * 1000.0,
        ));
    }
    all_stages.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    md.push_str("Fasi ordinate per tempo assoluto (dalla più lenta):\n\n");
    md.push_str("| Fase | Tempo (ms) |\n|---|---|\n");
    for (name, ms) in all_stages.iter().take(10) {
        md.push_str(&format!("| {name} | {ms:.0} |\n"));
    }
    if let Some((name, ms)) = all_stages.first() {
        md.push_str(&format!("\n**Fase più lenta in assoluto**: {name} ({ms:.0} ms) — è il primo posto da guardare per ottimizzare.\n"));
    }

    std::fs::write(&path, md).with_context(|| format!("scrittura report {}", path.display()))?;
    Ok(path)
}

// ── Modalità live (--bench-live) ────────────────────────────────────────────

/// Registra tempi/hardware di ogni ingestione e query REALI fatte da
/// frontend durante la sessione — a differenza di `--bench <file>`, che
/// misura un unico documento sintetico. Costruito una volta all'avvio se
/// `--bench-live` è passato (vedi main.rs) e condiviso via
/// `AppState::live_bench`. Il report è scritto alla chiusura del server
/// (SIGINT/SIGTERM, vedi main.rs) — niente endpoint dedicato nell'MVP.
pub struct LiveRecorder {
    hardware: HardwareInfo,
    started_at: String,
    ingestions: Mutex<Vec<LiveIngestion>>,
    inferences: Mutex<Vec<LiveInference>>,
}

struct LiveIngestion {
    at: String,
    filename: String,
    result: IngestionResult,
}

struct LiveInference {
    at: String,
    result: InferenceResult,
}

impl LiveRecorder {
    pub fn new(embeddings: &EmbeddingService, eullm_model: &str) -> Self {
        Self {
            hardware: collect_hardware_info(embeddings, eullm_model),
            started_at: now_string(),
            ingestions: Mutex::new(Vec::new()),
            inferences: Mutex::new(Vec::new()),
        }
    }

    /// Mai fallire la richiesta reale per colpa della registrazione: un
    /// lock avvelenato scarta l'evento con un warning invece di propagare
    /// un errore al chiamante (upload/query reali dell'utente).
    pub fn record_ingestion(&self, filename: String, result: IngestionResult) {
        match self.ingestions.lock() {
            Ok(mut v) => v.push(LiveIngestion { at: now_string(), filename, result }),
            Err(_) => tracing::warn!("bench-live: lock ingestioni avvelenato, evento perso"),
        }
    }

    pub fn record_inference(&self, result: InferenceResult) {
        match self.inferences.lock() {
            Ok(mut v) => v.push(LiveInference { at: now_string(), result }),
            Err(_) => tracing::warn!("bench-live: lock inferenze avvelenato, evento perso"),
        }
    }

    /// Scrive il report aggregato. `Ok(None)` se non è stata registrata
    /// nessuna ingestione/query (server avviato e chiuso senza uso reale nel
    /// frattempo) — niente report vuoto a confondere un run reale.
    pub fn write_report(&self) -> Result<Option<PathBuf>> {
        let ingestions = self
            .ingestions
            .lock()
            .map_err(|_| anyhow::anyhow!("bench-live: lock ingestioni avvelenato"))?;
        let inferences = self
            .inferences
            .lock()
            .map_err(|_| anyhow::anyhow!("bench-live: lock inferenze avvelenato"))?;
        if ingestions.is_empty() && inferences.is_empty() {
            return Ok(None);
        }
        write_live_report(&self.hardware, &self.started_at, &ingestions, &inferences).map(Some)
    }
}

fn now_string() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn avg_ms<'a>(durations: impl Iterator<Item = &'a Duration>) -> f64 {
    let (sum, n) = durations.fold((0.0_f64, 0usize), |(s, n), d| (s + d.as_secs_f64() * 1000.0, n + 1));
    if n == 0 { 0.0 } else { sum / n as f64 }
}

fn write_live_report(
    hw: &HardwareInfo,
    started_at: &str,
    ingestions: &[LiveIngestion],
    inferences: &[LiveInference],
) -> Result<PathBuf> {
    let now = now_string();
    let path = PathBuf::from(format!(
        "benchmark-live-report-{}.md",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));

    let mut md = String::new();
    md.push_str(&format!(
        "# Benchmark live i3k-rag-engine — sessione avviata {started_at}, report generato {now}\n\n"
    ));
    md.push_str(&format!(
        "Registrate {} ingestioni e {} query reali durante la sessione (uso da frontend, carico non \
         controllato — per confrontare hardware diversi a parità di carico usa `--bench <file>`, non questa modalità).\n\n",
        ingestions.len(),
        inferences.len()
    ));

    md.push_str("## Hardware\n\n");
    md.push_str("| Componente | Dettaglio |\n|---|---|\n");
    md.push_str(&format!("| CPU | {} ({} core) |\n", hw.cpu_model, hw.cpu_cores));
    md.push_str(&format!("| RAM | {} MB |\n", hw.ram_total_mb));
    match (&hw.gpu_name, hw.gpu_vram_total_mb, hw.gpu_vram_free_mb) {
        (Some(name), Some(total), Some(free)) => {
            md.push_str(&format!("| GPU | {name} |\n"));
            md.push_str(&format!("| VRAM | {free} MB liberi / {total} MB totali (all'avvio) |\n"));
        }
        _ => md.push_str("| GPU | non rilevata (nvidia-smi non disponibile o nessuna GPU) |\n"),
    }
    md.push_str(&format!("| OS | {} |\n", hw.os));
    md.push_str(&format!("| Device embedding | {} |\n", hw.embedding_device));
    md.push_str(&format!("| Modello eullm | {} |\n", hw.eullm_model));

    let mut bottleneck_stages: Vec<(&str, f64)> = Vec::new();

    if !ingestions.is_empty() {
        md.push_str("\n## Ingestioni\n\n");
        md.push_str(
            "| Ora | File | Pagine | Parole | Chunk | Estrazione (ms) | Chunking (ms) | Embedding (ms) | Upsert (ms) | Totale (ms) |\n|---|---|---|---|---|---|---|---|---|---|\n",
        );
        for ing in ingestions {
            let stage_ms = |name: &str| {
                ing.result.stages.iter().find(|s| s.name == name).map(|s| s.ms()).unwrap_or(0.0)
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |\n",
                ing.at,
                ing.filename,
                hw_opt(ing.result.page_count),
                ing.result.word_count,
                ing.result.chunk_count,
                stage_ms("Estrazione testo"),
                stage_ms("Chunking"),
                stage_ms("Embedding"),
                stage_ms("Upsert Qdrant"),
                ing.result.total().as_secs_f64() * 1000.0,
            ));
        }

        let by_stage = |name: &'static str| {
            avg_ms(
                ingestions
                    .iter()
                    .filter_map(move |i| i.result.stages.iter().find(|s| s.name == name))
                    .map(|s| &s.duration),
            )
        };
        let extract_avg = by_stage("Estrazione testo");
        let chunk_avg = by_stage("Chunking");
        let embed_avg = by_stage("Embedding");
        let upsert_avg = by_stage("Upsert Qdrant");

        md.push_str(&format!(
            "\n**Medie su {} ingestioni**: estrazione {extract_avg:.0} ms, chunking {chunk_avg:.0} ms, embedding {embed_avg:.0} ms, upsert {upsert_avg:.0} ms — totale medio {:.0} ms.\n",
            ingestions.len(),
            extract_avg + chunk_avg + embed_avg + upsert_avg
        ));

        md.push_str("\n```mermaid\npie title Tempo medio di ingestione per fase\n");
        md.push_str(&format!("    \"Estrazione testo\" : {:.1}\n", extract_avg.max(0.1)));
        md.push_str(&format!("    \"Chunking\" : {:.1}\n", chunk_avg.max(0.1)));
        md.push_str(&format!("    \"Embedding\" : {:.1}\n", embed_avg.max(0.1)));
        md.push_str(&format!("    \"Upsert Qdrant\" : {:.1}\n", upsert_avg.max(0.1)));
        md.push_str("```\n");

        bottleneck_stages.push(("Ingestione: estrazione testo", extract_avg));
        bottleneck_stages.push(("Ingestione: chunking", chunk_avg));
        bottleneck_stages.push(("Ingestione: embedding", embed_avg));
        bottleneck_stages.push(("Ingestione: upsert Qdrant", upsert_avg));
    }

    if !inferences.is_empty() {
        md.push_str("\n## Query\n\n");
        md.push_str(
            "| Ora | Domanda | Chunk trovati | Embed query (ms) | Ricerca (ms) | Prompt (ms) | TTFT (ms) | Generazione tot (ms) | Token | Decode tok/s |\n|---|---|---|---|---|---|---|---|---|---|\n",
        );
        for inf in inferences {
            let tps = inf
                .result
                .decode_tokens_per_sec()
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "N/A".to_owned());
            md.push_str(&format!(
                "| {} | {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {} | {} |\n",
                inf.at,
                truncate(&inf.result.query, 50),
                inf.result.chunks_retrieved,
                inf.result.embed_query.as_secs_f64() * 1000.0,
                inf.result.search.as_secs_f64() * 1000.0,
                inf.result.prompt_build.as_secs_f64() * 1000.0,
                inf.result.ttft.as_secs_f64() * 1000.0,
                inf.result.total_generation.as_secs_f64() * 1000.0,
                inf.result.tokens_generated,
                tps,
            ));
        }

        let embed_avg = avg_ms(inferences.iter().map(|i| &i.result.embed_query));
        let search_avg = avg_ms(inferences.iter().map(|i| &i.result.search));
        let prompt_avg = avg_ms(inferences.iter().map(|i| &i.result.prompt_build));
        let ttft_avg = avg_ms(inferences.iter().map(|i| &i.result.ttft));
        let decode_durations: Vec<Duration> = inferences
            .iter()
            .map(|i| i.result.total_generation.saturating_sub(i.result.ttft))
            .collect();
        let decode_avg = avg_ms(decode_durations.iter());
        let tps_values: Vec<f64> =
            inferences.iter().filter_map(|i| i.result.decode_tokens_per_sec()).collect();
        let tps_avg = if tps_values.is_empty() {
            None
        } else {
            Some(tps_values.iter().sum::<f64>() / tps_values.len() as f64)
        };

        md.push_str(&format!(
            "\n**Medie su {} query**: embed query {embed_avg:.0} ms, ricerca {search_avg:.0} ms, prompt {prompt_avg:.0} ms, TTFT {ttft_avg:.0} ms, decode {decode_avg:.0} ms",
            inferences.len()
        ));
        match tps_avg {
            Some(v) => md.push_str(&format!(", velocità decode media {v:.1} token/sec.\n")),
            None => md.push_str(" (velocità decode: N/A, troppo pochi token per query).\n"),
        }

        md.push_str("\n```mermaid\npie title Tempo medio di inferenza per fase\n");
        md.push_str(&format!("    \"Embedding query\" : {:.1}\n", embed_avg.max(0.1)));
        md.push_str(&format!("    \"Ricerca Qdrant\" : {:.1}\n", search_avg.max(0.1)));
        md.push_str(&format!("    \"Costruzione prompt\" : {:.1}\n", prompt_avg.max(0.1)));
        md.push_str(&format!("    \"Prefill (TTFT)\" : {:.1}\n", ttft_avg.max(0.1)));
        md.push_str(&format!("    \"Decode\" : {:.1}\n", decode_avg.max(0.1)));
        md.push_str("```\n");

        bottleneck_stages.push(("Query: embedding", embed_avg));
        bottleneck_stages.push(("Query: ricerca Qdrant", search_avg));
        bottleneck_stages.push(("Query: costruzione prompt", prompt_avg));
        bottleneck_stages.push(("Query: prefill/TTFT", ttft_avg));
        bottleneck_stages.push(("Query: decode", decode_avg));
    }

    if !bottleneck_stages.is_empty() {
        md.push_str("\n## Riepilogo colli di bottiglia\n\n");
        bottleneck_stages.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        md.push_str("Fasi ordinate per tempo MEDIO assoluto (dalla più lenta):\n\n");
        md.push_str("| Fase | Tempo medio (ms) |\n|---|---|\n");
        for (name, ms) in &bottleneck_stages {
            md.push_str(&format!("| {name} | {ms:.0} |\n"));
        }
        if let Some((name, ms)) = bottleneck_stages.first() {
            md.push_str(&format!(
                "\n**Fase più lenta in media**: {name} ({ms:.0} ms) — è il primo posto da guardare per ottimizzare.\n"
            ));
        }
    }

    std::fs::write(&path, md).with_context(|| format!("scrittura report {}", path.display()))?;
    Ok(path)
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub async fn run(
    settings: &Settings,
    args: &BenchArgs,
    embeddings: &mut EmbeddingService,
    eullm: std::sync::Arc<EullmClient>,
) -> Result<()> {
    if !args.doc_path.is_file() {
        anyhow::bail!("file non trovato: {}", args.doc_path.display());
    }

    let bench_collection = format!("{}_benchmark", settings.qdrant.collection);
    tracing::info!(collection = %bench_collection, "azzero la collection di benchmark");
    reset_benchmark_collection(&settings.qdrant.grpc_url, &bench_collection).await?;
    let qdrant = QdrantStore::new(&settings.qdrant.grpc_url, &bench_collection)
        .await
        .context("init Qdrant benchmark")?;

    tracing::info!(doc = %args.doc_path.display(), "avvio benchmark ingestione");
    // Stesso ordine di api/documents.rs::upload() sull'ingestione reale:
    // libera VRAM (eullm) PRIMA di occuparla (bge-m3), rientro speculare a
    // fine ingestione. Senza unload_during_ingestion, eullm resta piazzato con
    // l'allocazione decisa al proprio avvio (il sizing avviene una volta, al
    // load, non si ridimensiona a caldo per far spazio) e lo
    // swap dell'embedding su GPU può non trovare VRAM sufficiente e
    // fallire silenziosamente (log "swap embedding fallito") — misurando di
    // nuovo il percorso sbagliato: CPU invece di GPU.
    let unload_enabled = settings.eullm.unload_during_ingestion;
    let swap_enabled = settings.embeddings.swap_during_ingestion;
    if unload_enabled {
        if let Err(e) = eullm.unload().await {
            tracing::error!(error = %e, "eullm: unload pre-ingestione fallito, procedo comunque (nessuna VRAM liberata)");
        }
    }
    if swap_enabled {
        swap_embedding_device(embeddings, true);
    }
    let ingestion_result = run_ingestion(&args.doc_path, settings, embeddings, &qdrant).await;
    if swap_enabled {
        swap_embedding_device(embeddings, false);
    }
    if unload_enabled {
        if let Err(e) = eullm.reload().await {
            tracing::error!(error = %e, "eullm: reload post-ingestione fallito — il modello potrebbe non essere in VRAM, verifica manualmente");
        }
    }
    let ingestion = ingestion_result?;

    let queries = if args.queries.is_empty() { default_queries() } else { args.queries.clone() };
    let mut inferences = Vec::with_capacity(queries.len());
    for q in &queries {
        tracing::info!(query = %q, "avvio benchmark inferenza");
        inferences.push(run_inference(q, &ingestion.document_id, embeddings, &qdrant, &eullm).await?);
    }

    let hw = collect_hardware_info(embeddings, &settings.eullm.model);
    print_summary(&hw, &ingestion, &inferences);
    let report_path = write_markdown_report(&hw, &args.doc_path, &ingestion, &inferences)?;
    println!("Report completo: {}", report_path.display());

    Ok(())
}

/// Stessa logica di AppState::swap_embeddings_to_gpu/_to_cpu, ma qui non
/// esiste un AppState (--bench <file> non ne costruisce uno: niente DB,
/// niente server). Un fallimento non è fatale — come in
/// api/documents.rs::upload(), si logga e si prosegue con il device
/// corrente piuttosto che abortire un benchmark già in corso.
fn swap_embedding_device(embeddings: &mut EmbeddingService, to_gpu: bool) {
    let model_id = embeddings.model_id().to_owned();
    let loaded = if to_gpu {
        tracing::info!(
            "swap_during_ingestion attivo: sposto l'embedding su GPU per l'ingestione del benchmark"
        );
        EmbeddingService::load_gpu_for_ingestion(&model_id)
    } else {
        tracing::info!("rimetto l'embedding su CPU dopo l'ingestione del benchmark");
        EmbeddingService::load_cpu_parked(&model_id)
    };
    match loaded {
        Ok(fresh) => *embeddings = fresh,
        Err(e) => tracing::warn!(
            error = %e, to_gpu, "swap embedding fallito, continuo con il device corrente"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_args_requires_bench_flag() {
        assert!(parse_args(&args(&["i3k-rag-engine"])).is_none());
    }

    #[test]
    fn parse_args_short_and_long_flag() {
        let a = parse_args(&args(&["i3k-rag-engine", "--bench", "doc.pdf"])).unwrap();
        assert_eq!(a.doc_path, PathBuf::from("doc.pdf"));
        assert!(a.queries.is_empty());

        let a = parse_args(&args(&["i3k-rag-engine", "--benchmark", "doc.pdf"])).unwrap();
        assert_eq!(a.doc_path, PathBuf::from("doc.pdf"));
    }

    #[test]
    fn parse_args_collects_repeated_queries() {
        let a = parse_args(&args(&[
            "i3k-rag-engine",
            "--bench",
            "doc.pdf",
            "--bench-query",
            "prima domanda",
            "--bench-query",
            "seconda domanda",
        ]))
        .unwrap();
        assert_eq!(a.queries, vec!["prima domanda".to_owned(), "seconda domanda".to_owned()]);
    }

    #[test]
    fn parse_args_missing_path_returns_none() {
        assert!(parse_args(&args(&["i3k-rag-engine", "--bench"])).is_none());
    }

    #[test]
    fn live_mode_requires_flag() {
        assert!(!live_mode_requested(&args(&["i3k-rag-engine"])));
        assert!(live_mode_requested(&args(&["i3k-rag-engine", "--bench-live"])));
    }

    #[test]
    fn avg_ms_empty_iterator_is_zero() {
        let v: Vec<Duration> = vec![];
        assert_eq!(avg_ms(v.iter()), 0.0);
    }

    #[test]
    fn avg_ms_computes_mean() {
        let v = [Duration::from_millis(100), Duration::from_millis(200), Duration::from_millis(300)];
        assert!((avg_ms(v.iter()) - 200.0).abs() < 0.01);
    }

    #[test]
    fn decode_tokens_per_sec_none_with_one_token() {
        let inf = InferenceResult {
            query: "q".into(),
            embed_query: Duration::ZERO,
            search: Duration::ZERO,
            prompt_build: Duration::ZERO,
            ttft: Duration::from_millis(500),
            total_generation: Duration::from_millis(500),
            tokens_generated: 1,
            chunks_retrieved: 0,
            chunks_from_bench_doc: 0,
        };
        assert!(inf.decode_tokens_per_sec().is_none());
    }

    #[test]
    fn decode_tokens_per_sec_computed_excluding_prefill() {
        let inf = InferenceResult {
            query: "q".into(),
            embed_query: Duration::ZERO,
            search: Duration::ZERO,
            prompt_build: Duration::ZERO,
            ttft: Duration::from_millis(500),
            total_generation: Duration::from_millis(1500), // 1000ms di decode
            tokens_generated: 11,                          // 10 intervalli
            chunks_retrieved: 0,
            chunks_from_bench_doc: 0,
        };
        // 10 intervalli in 1000ms = 10 token/sec
        assert!((inf.decode_tokens_per_sec().unwrap() - 10.0).abs() < 0.01);
    }
}
