# i3k RAG Engine — Community

Open-source Rust rewrite of the RAG Enterprise Python backend, replacing it as
of v2. A single binary: no Java, no Docker Compose, no orchestration — it
downloads and verifies its own components, bundles the compiled frontend, and
serves the API.

The Python version this replaces is preserved on the
[`python-legacy`](../../tree/python-legacy) branch and in the `1.x` tags. It is
no longer developed, but it remains available for anyone still running it.

## Architecture

- **Embedding**: `BAAI/bge-m3` (1024-dim, CLS pooling, L2-normalized) via [Candle](https://github.com/huggingface/candle) — in-process, no cuDNN
- **LLM**: [eullm](https://github.com/eullm/eullm) (separate process) via `POST /api/generate` raw mode + no-think prefill
- **Vector DB**: Qdrant (collection `rag_documents`, cosine distance)
- **SQL**: SQLite via sqlx (async, compile-checked; Postgres-ready)
- **Ingest**: pdf_oxide (text) + pdfium-render + leptess/Tesseract (scanned pages)
- **API**: axum + JWT auth (HS256)

## Status

Phase 1 in progress.

## Prerequisites

- Rust 1.75+
- Qdrant running locally (default: `http://localhost:6333`)
- eullm running locally (default: `http://localhost:11434`)
- Tesseract OCR with `ita` and `eng` traineddata
- pdfium shared library

## Build

```sh
cargo build --release
```

## License

MIT OR Apache-2.0
