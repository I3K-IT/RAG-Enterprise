# i3k RAG Engine — Community

Open-source Rust port of the [RAG Enterprise](https://github.com/i3k-it/RAG-Enterprise) Python backend.

## Architecture

- **Embedding**: `BAAI/bge-m3` (1024-dim, CLS pooling, L2-normalized) via [Candle](https://github.com/huggingface/candle) — in-process, no cuDNN
- **LLM**: [eullm](https://github.com/eullm/eullm) (separate process) via `POST /api/generate` raw mode + no-think prefill
- **Vector DB**: Qdrant (collection `rag_documents`, cosine distance)
- **SQL**: SQLite via sqlx (async, compile-checked; Postgres-ready)
- **Ingest**: pdf_oxide (text) + pdfium-render + leptess/Tesseract (scanned pages)
- **API**: axum + JWT auth (HS256)

## Status

Phase 1 in progress — see [PIANO_i3kragengine.md](PIANO_i3kragengine.md) for the roadmap.

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
