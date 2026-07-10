//! --bench/--benchmark: misura ingestione+inferenza su un documento reale,
//! con l'hardware effettivamente in uso, e scrive un report Markdown (con
//! grafici a torta Mermaid) pensato per individuare colli di bottiglia.
//!
//! Riusa lo stesso bootstrap (qdrant/eullm/embedding) dell'avvio normale —
//! niente scorciatoie: i tempi devono riflettere condizioni reali. Scrive in
//! una collection Qdrant dedicata ("{collection}_benchmark", azzerata ad ogni
//! run) per non toccare mai i dati reali dell'utente.

use std::path::{Path, PathBuf};
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

fn default_queries() -> Vec<String> {
    vec![
        "Riassumi in breve il contenuto di questo documento.".to_owned(),
        "Quali sono i punti principali trattati nel documento?".to_owned(),
    ]
}

// ── Hardware ─────────────────────────────────────────────────────────────────

struct HardwareInfo {
    cpu_model: String,
    cpu_cores: usize,
    ram_total_mb: u64,
    gpu_name: Option<String>,
    gpu_vram_total_mb: Option<u64>,
    gpu_vram_free_mb: Option<u64>,
    os: String,
    embedding_device: String,
    eullm_model: String,
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

fn collect_hardware_info(embeddings: &EmbeddingService, eullm_model: &str) -> HardwareInfo {
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

struct StageTiming {
    name: &'static str,
    duration: Duration,
}

impl StageTiming {
    fn ms(&self) -> f64 {
        self.duration.as_secs_f64() * 1000.0
    }
}

struct IngestionResult {
    document_id: String,
    stages: Vec<StageTiming>,
    page_count: Option<u32>,
    word_count: usize,
    char_count: usize,
    chunk_count: usize,
}

impl IngestionResult {
    fn total(&self) -> Duration {
        self.stages.iter().map(|s| s.duration).sum()
    }
}

struct InferenceResult {
    query: String,
    embed_query: Duration,
    search: Duration,
    prompt_build: Duration,
    ttft: Duration,
    total_generation: Duration,
    tokens_generated: usize,
    chunks_retrieved: usize,
    chunks_from_bench_doc: usize,
}

impl InferenceResult {
    /// token/sec nella sola fase di decode (esclude il prefill/TTFT) — è la
    /// velocità "di regime", quella che conta per risposte lunghe. Con un
    /// solo token generato non è definita (nessun intervallo decode misurabile).
    fn decode_tokens_per_sec(&self) -> Option<f64> {
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

    let word_count = text.split_whitespace().count();
    let char_count = text.chars().count();

    let t = Instant::now();
    let chunks = chunker::split_text(&text);
    let chunk_time = t.elapsed();
    if chunks.is_empty() {
        anyhow::bail!("nessun testo estratto da {} — impossibile fare benchmark", doc_path.display());
    }

    let t = Instant::now();
    let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
    let embedding_vecs = embeddings.embed_texts(&chunk_refs).context("embedding chunk")?;
    let embed_time = t.elapsed();

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

    let chunks_from_bench_doc = hits.iter().filter(|h| h.payload.document_id == bench_document_id).count();

    let context: String = hits
        .iter()
        .map(|h| format!("[{}]\n{}", h.payload.filename, h.payload.text))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let t = Instant::now();
    let full_prompt = prompt::build_prompt(&context, query, &[]);
    let prompt_build = t.elapsed();

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
        }
        tokens_generated += 1;
    }
    let total_generation = gen_start.elapsed();
    gen_handle.await.context("join generazione eullm")?.context("eullm invoke_stream")?;

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

// ── Entry point ──────────────────────────────────────────────────────────────

pub async fn run(
    settings: &Settings,
    args: &BenchArgs,
    embeddings: &EmbeddingService,
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
    let ingestion = run_ingestion(&args.doc_path, settings, embeddings, &qdrant).await?;

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
