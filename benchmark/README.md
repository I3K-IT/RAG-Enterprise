# RAG Enterprise Benchmark

Standardized benchmark for comparing RAG performance across different hardware configurations.

## Quick Start

```bash
# From the benchmark directory
cd benchmark

# Run benchmark (requires running RAG system)
python rag_benchmark.py --api-url http://localhost:8000

# With custom credentials
python rag_benchmark.py --api-url http://localhost:8000 --username admin --password yourpassword

# Custom output directory
python rag_benchmark.py --output ./my_results
```

## What It Tests

### Documents (Public Domain)
| Document | Type | Pages | Tests |
|----------|------|-------|-------|
| US Constitution | Legal | ~15 | Government structure queries |
| UN Human Rights Declaration | Legal | ~10 | Article lookups |
| Bitcoin Whitepaper | Technical | 9 | Technical concept queries |
| RFC 2616 (HTTP/1.1) | Technical | ~170 | Protocol specification queries |
| Einstein - Relativity | Scientific | ~100 | Scientific concept queries |

### Metrics Measured
- **Upload Performance**: Time to upload and index documents
- **Query Latency**: Response time for RAG queries
- **Retrieval Quality**: Similarity scores of retrieved chunks
- **Hardware Utilization**: CPU, RAM, GPU info

## Output

Results are saved in two formats:

### JSON (`benchmark_YYYYMMDD_HHMMSS.json`)
Complete raw data for programmatic analysis.

### Markdown (`benchmark_YYYYMMDD_HHMMSS.md`)
Human-readable report with tables and summary.

## Sharing Results

We encourage users to share their benchmark results to build a hardware compatibility database.

To contribute:
1. Run the benchmark on your hardware
2. Open an issue or PR with your results
3. Include the Markdown report and hardware details

## Requirements

```bash
pip install requests
```

The benchmark script has minimal dependencies to run outside Docker.

## Options

| Option | Default | Description |
|--------|---------|-------------|
| `--api-url` | http://localhost:8000 | RAG API endpoint |
| `--output` | ./benchmark_results | Output directory |
| `--username` | admin | API username |
| `--password` | admin | API password |
| `--no-clear` | false | Don't clear existing documents |
