//! --bench/--benchmark: measures ingestion and inference on a real document,
//! on the hardware actually in use, and writes a Markdown report (with Mermaid
//! pie charts) meant for spotting bottlenecks.
//!
//! It reuses the same bootstrap (qdrant, eullm, embeddings) as a normal start,
//! with no shortcuts: the timings must reflect real conditions. It writes into
//! a dedicated Qdrant collection ("{collection}_benchmark", wiped on every
//! run) so the user's real data is never touched.

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

/// `--bench <path>` or `--benchmark <path>`, with zero or more repeated
/// `--bench-query "…"`. When no query is given, run() uses a small generic
/// set.
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

/// `--bench-live`: an alternative to `--bench <file>`. The server and
/// frontend start normally and EVERY real ingestion and query made during the
/// session is timed and recorded (see LiveRecorder), with the report written
/// on shutdown (SIGINT/SIGTERM, see main.rs).
///
/// Unlike `--bench <file>` the workload is not fixed: this is useful for
/// understanding behaviour under real use, not for comparing different
/// hardware at equal load — for that you want `--bench <file>`, which is
/// reproducible.
pub fn live_mode_requested(args: &[String]) -> bool {
    args.iter().any(|a| a == "--bench-live")
}

fn default_queries() -> Vec<String> {
    vec![
        "Briefly summarise the contents of this document.".to_owned(),
        "What are the main points covered in the document?".to_owned(),
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

/// Runs a one-line PowerShell script and returns its trimmed stdout. Used
/// only for the three Windows hardware queries below — `--bench` runs this
/// once per session, not per query, so the process-spawn cost of PowerShell
/// (heavier than a /proc read) does not matter.
#[cfg(windows)]
fn powershell(script: &str) -> Option<String> {
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(not(windows))]
fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}
#[cfg(windows)]
fn cpu_model() -> String {
    powershell("(Get-CimInstance Win32_Processor).Name").unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(not(windows))]
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
#[cfg(windows)]
fn ram_total_mb() -> u64 {
    // TotalPhysicalMemory is bytes, unlike /proc/meminfo's MemTotal (KiB).
    powershell("(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory")
        .and_then(|s| s.parse::<u64>().ok())
        .map(|bytes| bytes / 1024 / 1024)
        .unwrap_or(0)
}

#[cfg(not(windows))]
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
#[cfg(windows)]
fn os_info() -> String {
    let script = r#"$os = Get-CimInstance Win32_OperatingSystem; "$($os.Caption) (build $($os.BuildNumber))""#;
    powershell(script).unwrap_or_else(|| format!("{} {}", std::env::consts::OS, std::env::consts::ARCH))
}

/// Uses nvidia-smi rather than the CUDA APIs directly: it works regardless of
/// how this binary was compiled (with or without --features cuda) and reflects
/// the REAL state of the GPU at benchmark time, since free VRAM changes
/// continuously.
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
    /// Tokens per second during the decode phase alone, excluding prefill and
    /// TTFT. This is the steady-state speed, the one that matters for long
    /// answers. With a single generated token it is undefined, since there is
    /// no measurable decode interval.
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
        .context("join text extraction")?
        .with_context(|| format!("text extraction from {}", doc_path.display()))?;
    let extract_time = t.elapsed();
    tracing::info!(
        pages = ?page_count, chars = text.chars().count(), ms = extract_time.as_millis(),
        "extraction complete"
    );

    let word_count = text.split_whitespace().count();
    let char_count = text.chars().count();

    let t = Instant::now();
    let chunks = chunker::split_text(&text);
    let chunk_time = t.elapsed();
    if chunks.is_empty() {
        anyhow::bail!("no text extracted from {} — cannot benchmark it", doc_path.display());
    }
    // Radio silence between here and the end of embedding can last minutes on
    // large documents (hundreds of chunks in a single embed_texts call), so
    // this log keeps it from looking stuck when it is merely working.
    tracing::info!(
        chunks = chunks.len(), ms = chunk_time.as_millis(),
        "chunking done, starting embedding (this can take minutes on large documents)"
    );

    let t = Instant::now();
    let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
    let embedding_vecs = embeddings.embed_texts(&chunk_refs).context("embedding chunk")?;
    let embed_time = t.elapsed();
    tracing::info!(ms = embed_time.as_millis(), "embedding done, starting the Qdrant upsert");

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
    tracing::info!(ms = upsert_time.as_millis(), "upsert complete, ingestion finished");

    Ok(IngestionResult {
        document_id,
        stages: vec![
            StageTiming { name: "Text extraction", duration: extract_time },
            StageTiming { name: "Chunking", duration: chunk_time },
            StageTiming { name: "Embedding", duration: embed_time },
            StageTiming { name: "Qdrant upsert", duration: upsert_time },
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

    tracing::info!("starting eullm generation");
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
            tracing::info!(ttft_ms = gen_start.elapsed().as_millis(), "first token received");
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

/// Wipes the benchmark collection before each run so results never mix with
/// those of earlier runs. Never the user's real collection. QdrantStore::new
/// recreates it from scratch when missing (see ensure_collection in
/// qdrant_store.rs).
async fn reset_benchmark_collection(grpc_url: &str, collection: &str) -> Result<()> {
    let client = Qdrant::from_url(grpc_url).build().context("connecting to Qdrant")?;
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
            println!("GPU: {name} | free VRAM: {free}/{total} MB")
        }
        _ => println!("GPU: not detected (nvidia-smi unavailable, or no GPU)"),
    }
    println!("Embedding device: {}", hw.embedding_device);
    println!();
    println!(
        "Ingestion — {} pages, {} words, {} chunks — {:.0} ms total",
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
        println!("Inference #{} — \"{}\"", i + 1, truncate(&inf.query, 60));
        println!(
            "  chunk recuperati: {} (di cui {} dal documento benchmark)",
            inf.chunks_retrieved, inf.chunks_from_bench_doc
        );
        println!("  TTFT: {:.0} ms | total generation: {:.0} ms | tokens: {}",
            inf.ttft.as_secs_f64() * 1000.0,
            inf.total_generation.as_secs_f64() * 1000.0,
            inf.tokens_generated
        );
        if let Some(tps) = inf.decode_tokens_per_sec() {
            println!("  decode speed: {tps:.1} tokens/sec");
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
    md.push_str(&format!("| CPU | {} ({} cores) |\n", hw.cpu_model, hw.cpu_cores));
    md.push_str(&format!("| RAM | {} MB |\n", hw.ram_total_mb));
    match (&hw.gpu_name, hw.gpu_vram_total_mb, hw.gpu_vram_free_mb) {
        (Some(name), Some(total), Some(free)) => {
            md.push_str(&format!("| GPU | {name} |\n"));
            md.push_str(&format!("| VRAM | {free} MB liberi / {total} MB totali |\n"));
        }
        _ => md.push_str("| GPU | not detected (nvidia-smi unavailable, or no GPU) |\n"),
    }
    md.push_str(&format!("| OS | {} |\n", hw.os));
    md.push_str(&format!("| Device embedding | {} |\n", hw.embedding_device));
    md.push_str(&format!("| eullm model | {} |\n", hw.eullm_model));

    md.push_str("\n## Document\n\n");
    md.push_str(&format!("- File: `{}`\n", doc_path.display()));
    md.push_str(&format!("- Pages: {}\n", hw_opt(ingestion.page_count)));
    md.push_str(&format!("- Words: {}\n", ingestion.word_count));
    md.push_str(&format!("- Caratteri: {}\n", ingestion.char_count));
    md.push_str(&format!("- Chunks produced: {}\n", ingestion.chunk_count));

    md.push_str("\n## Ingestion — time per stage\n\n");
    md.push_str("| Stage | Time (ms) | % of total |\n|---|---|---|\n");
    let ingestion_total_ms = (ingestion.total().as_secs_f64() * 1000.0).max(0.001);
    for s in &ingestion.stages {
        md.push_str(&format!("| {} | {:.0} | {:.1}% |\n", s.name, s.ms(), s.ms() / ingestion_total_ms * 100.0));
    }
    md.push_str(&format!("| **Total** | **{:.0}** | **100%** |\n", ingestion_total_ms));

    md.push_str("\n```mermaid\npie title Ingestion time per stage\n");
    for s in &ingestion.stages {
        md.push_str(&format!("    \"{}\" : {:.1}\n", s.name, s.ms().max(0.1)));
    }
    md.push_str("```\n");

    if let Some(worst) = ingestion.stages.iter().max_by(|a, b| a.duration.cmp(&b.duration)) {
        md.push_str(&format!(
            "\n**Ingestion bottleneck**: {} ({:.1}% of total time)\n",
            worst.name,
            worst.ms() / ingestion_total_ms * 100.0
        ));
    }

    md.push_str("\n## Inference\n\n");
    for (i, inf) in inferences.iter().enumerate() {
        md.push_str(&format!("### Query {}: \"{}\"\n\n", i + 1, inf.query));
        md.push_str("| Stage | Time (ms) |\n|---|---|\n");
        md.push_str(&format!("| Embedding query | {:.0} |\n", inf.embed_query.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Qdrant search | {:.0} |\n", inf.search.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Costruzione prompt | {:.0} |\n", inf.prompt_build.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Time to first token (TTFT / prefill) | {:.0} |\n", inf.ttft.as_secs_f64() * 1000.0));
        md.push_str(&format!("| Total generation | {:.0} |\n", inf.total_generation.as_secs_f64() * 1000.0));

        md.push_str(&format!(
            "\n- Chunks retrieved: {} (top_k={}, similarity threshold ≥ {}), of which {} from the document just ingested\n",
            inf.chunks_retrieved, retrieval::TOP_K, retrieval::RELEVANCE_THRESHOLD, inf.chunks_from_bench_doc
        ));
        md.push_str(&format!("- Token generati: {}\n", inf.tokens_generated));
        match inf.decode_tokens_per_sec() {
            Some(tps) => md.push_str(&format!("- Decode speed (excluding prefill): {tps:.1} tokens/sec\n")),
            None => md.push_str("- Decode speed: N/A (too few tokens generated)\n"),
        }

        let decode_ms = inf.total_generation.saturating_sub(inf.ttft).as_secs_f64() * 1000.0;
        md.push_str("\n```mermaid\npie title Inference time per stage\n");
        md.push_str(&format!("    \"Embedding query\" : {:.1}\n", (inf.embed_query.as_secs_f64() * 1000.0).max(0.1)));
        md.push_str(&format!("    \"Ricerca Qdrant\" : {:.1}\n", (inf.search.as_secs_f64() * 1000.0).max(0.1)));
        md.push_str(&format!("    \"Prefill (TTFT)\" : {:.1}\n", (inf.ttft.as_secs_f64() * 1000.0).max(0.1)));
        md.push_str(&format!("    \"Decode\" : {:.1}\n", decode_ms.max(0.1)));
        md.push_str("```\n\n");
    }

    md.push_str("## Bottleneck summary\n\n");
    let mut all_stages: Vec<(String, f64)> = ingestion
        .stages
        .iter()
        .map(|s| (format!("Ingestion: {}", s.name), s.ms()))
        .collect();
    for (i, inf) in inferences.iter().enumerate() {
        all_stages.push((format!("Query {}: embedding", i + 1), inf.embed_query.as_secs_f64() * 1000.0));
        all_stages.push((format!("Query {}: Qdrant search", i + 1), inf.search.as_secs_f64() * 1000.0));
        all_stages.push((format!("Query {}: prefill/TTFT", i + 1), inf.ttft.as_secs_f64() * 1000.0));
        all_stages.push((
            format!("Query {}: decode", i + 1),
            inf.total_generation.saturating_sub(inf.ttft).as_secs_f64() * 1000.0,
        ));
    }
    all_stages.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    md.push_str("Stages by absolute time, slowest first:\n\n");
    md.push_str("| Stage | Time (ms) |\n|---|---|\n");
    for (name, ms) in all_stages.iter().take(10) {
        md.push_str(&format!("| {name} | {ms:.0} |\n"));
    }
    if let Some((name, ms)) = all_stages.first() {
        md.push_str(&format!("\n**Slowest stage overall**: {name} ({ms:.0} ms) — the first place to look when optimising.\n"));
    }

    std::fs::write(&path, md).with_context(|| format!("writing report {}", path.display()))?;
    Ok(path)
}

// ── Modalità live (--bench-live) ────────────────────────────────────────────

/// Records timings and hardware for every REAL ingestion and query made from
/// the frontend during the session — unlike `--bench <file>`, which measures a
/// single synthetic document.
/// Built once at startup when `--bench-live` is passed (see main.rs) and
/// shared through `AppState::live_bench`. The report is written when the
/// server shuts down (SIGINT/SIGTERM, see main.rs) — there is no dedicated
/// endpoint in the MVP.
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

    /// Never fail a real request because of the recording: a poisoned lock
    /// drops the event with a warning instead of propagating an error to the
    /// caller, which would be the user's real upload or query.
    pub fn record_ingestion(&self, filename: String, result: IngestionResult) {
        match self.ingestions.lock() {
            Ok(mut v) => v.push(LiveIngestion { at: now_string(), filename, result }),
            Err(_) => tracing::warn!("bench-live: ingestion lock poisoned, event lost"),
        }
    }

    pub fn record_inference(&self, result: InferenceResult) {
        match self.inferences.lock() {
            Ok(mut v) => v.push(LiveInference { at: now_string(), result }),
            Err(_) => tracing::warn!("bench-live: inference lock poisoned, event lost"),
        }
    }

    /// Writes the aggregated report. `Ok(None)` when no ingestion or query was
    /// recorded — the server was started and stopped without real use in
    /// between — so an empty report never gets mistaken for a real run.
    pub fn write_report(&self) -> Result<Option<PathBuf>> {
        let ingestions = self
            .ingestions
            .lock()
            .map_err(|_| anyhow::anyhow!("bench-live: ingestion lock poisoned"))?;
        let inferences = self
            .inferences
            .lock()
            .map_err(|_| anyhow::anyhow!("bench-live: inference lock poisoned"))?;
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
        "# i3k-rag-engine live benchmark — session started {started_at}, report generated {now}\n\n"
    ));
    md.push_str(&format!(
        "Recorded {} real ingestions and {} real queries during the session (frontend use, so the \
         load is uncontrolled — to compare different hardware at equal load use `--bench <file>`, not this mode).\n\n",
        ingestions.len(),
        inferences.len()
    ));

    md.push_str("## Hardware\n\n");
    md.push_str("| Componente | Dettaglio |\n|---|---|\n");
    md.push_str(&format!("| CPU | {} ({} cores) |\n", hw.cpu_model, hw.cpu_cores));
    md.push_str(&format!("| RAM | {} MB |\n", hw.ram_total_mb));
    match (&hw.gpu_name, hw.gpu_vram_total_mb, hw.gpu_vram_free_mb) {
        (Some(name), Some(total), Some(free)) => {
            md.push_str(&format!("| GPU | {name} |\n"));
            md.push_str(&format!("| VRAM | {free} MB free / {total} MB total (at startup) |\n"));
        }
        _ => md.push_str("| GPU | not detected (nvidia-smi unavailable, or no GPU) |\n"),
    }
    md.push_str(&format!("| OS | {} |\n", hw.os));
    md.push_str(&format!("| Device embedding | {} |\n", hw.embedding_device));
    md.push_str(&format!("| eullm model | {} |\n", hw.eullm_model));

    let mut bottleneck_stages: Vec<(&str, f64)> = Vec::new();

    if !ingestions.is_empty() {
        md.push_str("\n## Ingestioni\n\n");
        md.push_str(
            "| Time | File | Pages | Words | Chunks | Extraction (ms) | Chunking (ms) | Embedding (ms) | Upsert (ms) | Total (ms) |\n|---|---|---|---|---|---|---|---|---|---|\n",
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
                stage_ms("Text extraction"),
                stage_ms("Chunking"),
                stage_ms("Embedding"),
                stage_ms("Qdrant upsert"),
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
        let extract_avg = by_stage("Text extraction");
        let chunk_avg = by_stage("Chunking");
        let embed_avg = by_stage("Embedding");
        let upsert_avg = by_stage("Qdrant upsert");

        md.push_str(&format!(
            "\n**Averages over {} ingestions**: extraction {extract_avg:.0} ms, chunking {chunk_avg:.0} ms, embedding {embed_avg:.0} ms, upsert {upsert_avg:.0} ms — {:.0} ms total on average.\n",
            ingestions.len(),
            extract_avg + chunk_avg + embed_avg + upsert_avg
        ));

        md.push_str("\n```mermaid\npie title Average ingestion time per stage\n");
        md.push_str(&format!("    \"Text extraction\" : {:.1}\n", extract_avg.max(0.1)));
        md.push_str(&format!("    \"Chunking\" : {:.1}\n", chunk_avg.max(0.1)));
        md.push_str(&format!("    \"Embedding\" : {:.1}\n", embed_avg.max(0.1)));
        md.push_str(&format!("    \"Upsert Qdrant\" : {:.1}\n", upsert_avg.max(0.1)));
        md.push_str("```\n");

        bottleneck_stages.push(("Ingestion: text extraction", extract_avg));
        bottleneck_stages.push(("Ingestion: chunking", chunk_avg));
        bottleneck_stages.push(("Ingestion: embedding", embed_avg));
        bottleneck_stages.push(("Ingestion: Qdrant upsert", upsert_avg));
    }

    if !inferences.is_empty() {
        md.push_str("\n## Query\n\n");
        md.push_str(
            "| Time | Question | Chunks found | Query embed (ms) | Search (ms) | Prompt (ms) | TTFT (ms) | Total generation (ms) | Tokens | Decode tok/s |\n|---|---|---|---|---|---|---|---|---|---|\n",
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
            Some(v) => md.push_str(&format!(", average decode speed {v:.1} tokens/sec.\n")),
            None => md.push_str(" (decode speed: N/A, too few tokens per query).\n"),
        }

        md.push_str("\n```mermaid\npie title Average inference time per stage\n");
        md.push_str(&format!("    \"Embedding query\" : {:.1}\n", embed_avg.max(0.1)));
        md.push_str(&format!("    \"Ricerca Qdrant\" : {:.1}\n", search_avg.max(0.1)));
        md.push_str(&format!("    \"Costruzione prompt\" : {:.1}\n", prompt_avg.max(0.1)));
        md.push_str(&format!("    \"Prefill (TTFT)\" : {:.1}\n", ttft_avg.max(0.1)));
        md.push_str(&format!("    \"Decode\" : {:.1}\n", decode_avg.max(0.1)));
        md.push_str("```\n");

        bottleneck_stages.push(("Query: embedding", embed_avg));
        bottleneck_stages.push(("Query: Qdrant search", search_avg));
        bottleneck_stages.push(("Query: prompt build", prompt_avg));
        bottleneck_stages.push(("Query: prefill/TTFT", ttft_avg));
        bottleneck_stages.push(("Query: decode", decode_avg));
    }

    if !bottleneck_stages.is_empty() {
        md.push_str("\n## Bottleneck summary\n\n");
        bottleneck_stages.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        md.push_str("Stages by average absolute time, slowest first:\n\n");
        md.push_str("| Stage | Average time (ms) |\n|---|---|\n");
        for (name, ms) in &bottleneck_stages {
            md.push_str(&format!("| {name} | {ms:.0} |\n"));
        }
        if let Some((name, ms)) = bottleneck_stages.first() {
            md.push_str(&format!(
                "\n**Slowest stage on average**: {name} ({ms:.0} ms) — the first place to look when optimising.\n"
            ));
        }
    }

    std::fs::write(&path, md).with_context(|| format!("writing report {}", path.display()))?;
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
        anyhow::bail!("file not found: {}", args.doc_path.display());
    }

    let bench_collection = format!("{}_benchmark", settings.qdrant.collection);
    tracing::info!(collection = %bench_collection, "azzero la collection di benchmark");
    reset_benchmark_collection(&settings.qdrant.grpc_url, &bench_collection).await?;
    let qdrant = QdrantStore::new(&settings.qdrant.grpc_url, &bench_collection)
        .await
        .context("init Qdrant benchmark")?;

    tracing::info!(doc = %args.doc_path.display(), "starting the ingestion benchmark");
    // Same order as api/documents.rs::upload() on a real ingestion: free the
    // VRAM (eullm) BEFORE occupying it (bge-m3), with the mirror image at the
    // end. Without unload_during_ingestion, eullm keeps the allocation decided
    // at its own startup — sizing happens once, at load, and is not resized
    // on the fly to make room — so swapping the embedding model onto the GPU
    // may not find enough VRAM and fail silently, which would once again
    // measure the wrong path: CPU instead of GPU.
    let unload_enabled = settings.eullm.unload_during_ingestion;
    // --bench only exercises the Candle swap path, not IngestionEmbedding::Eullm:
    // it runs without an AppState (no database, no server), so there is nothing
    // for documents::upload()'s eullm-embed branch to plug into here.
    let swap_enabled = settings.embeddings.ingestion_embedding
        == crate::config::IngestionEmbedding::CandleGpu;
    if unload_enabled {
        if let Err(e) = eullm.unload().await {
            tracing::error!(error = %e, "eullm: unload before ingestion failed, carrying on anyway (no VRAM freed)");
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
            tracing::error!(error = %e, "eullm: reload after ingestion failed — the model may not be resident in VRAM, check manually");
        }
    }
    let ingestion = ingestion_result?;

    let queries = if args.queries.is_empty() { default_queries() } else { args.queries.clone() };
    let mut inferences = Vec::with_capacity(queries.len());
    for q in &queries {
        tracing::info!(query = %q, "starting the inference benchmark");
        inferences.push(run_inference(q, &ingestion.document_id, embeddings, &qdrant, &eullm).await?);
    }

    let hw = collect_hardware_info(embeddings, &settings.eullm.model);
    print_summary(&hw, &ingestion, &inferences);
    let report_path = write_markdown_report(&hw, &args.doc_path, &ingestion, &inferences)?;
    println!("Report completo: {}", report_path.display());

    Ok(())
}

/// Same logic as AppState::swap_embeddings_to_gpu/_to_cpu, except there is no
/// AppState here: `--bench <file>` never builds one, since it runs without a
/// database or a server. A failure is not fatal — as in
/// api/documents.rs::upload(), it is logged and the current device is kept
/// rather than aborting a benchmark already under way.
fn swap_embedding_device(embeddings: &mut EmbeddingService, to_gpu: bool) {
    let model_id = embeddings.model_id().to_owned();
    let loaded = if to_gpu {
        tracing::info!(
            "ingestion_embedding=candle_gpu: moving the embedding model to GPU for the benchmark ingestion"
        );
        EmbeddingService::load_gpu_for_ingestion(&model_id)
    } else {
        tracing::info!("moving the embedding model back to CPU after the benchmark ingestion");
        EmbeddingService::load_cpu_parked(&model_id)
    };
    match loaded {
        Ok(fresh) => *embeddings = fresh,
        Err(e) => tracing::warn!(
            error = %e, to_gpu, "embedding swap failed, continuing on the current device"
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
            "first question",
            "--bench-query",
            "second question",
        ]))
        .unwrap();
        assert_eq!(a.queries, vec!["first question".to_owned(), "second question".to_owned()]);
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
