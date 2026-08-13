# i3k RAG Engine

**Ask questions about your own documents. Everything runs on your machine.**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![Release](https://img.shields.io/github/v/release/I3K-IT/RAG-Enterprise)](../../releases)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

A self-hosted Retrieval-Augmented Generation system: upload your documents, ask
questions in plain language, get answers grounded in what those documents
actually say, with the sources cited. No data ever leaves your server — there
is no telemetry, and nothing is sent to an external API.

> **v2 is a complete rewrite in Rust.** The Python version this replaces is
> preserved on the [`python-legacy`](../../tree/python-legacy) branch and in the
> `1.x` tags. See [Upgrading from 1.x](#upgrading-from-1x).

## Why

Most RAG systems either send your documents to somebody else's API, or ask you
to orchestrate five containers before you can ask a single question. This does
neither.

- **One binary.** No Docker, no Compose, no Java. It downloads and
  sha256-verifies everything it needs on first run — vector database, inference
  engine, models, OCR data — then runs them itself.
- **Genuinely offline.** After the first run it needs no network at all.
- **Auditable.** Apache-2.0, and you can read every line that touches your data.

Built for people who cannot use cloud RAG for regulatory, privacy or
data-sovereignty reasons: law firms, healthcare, finance, public administration.

## Quick start

Download the tarball for your platform from the [releases](../../releases) page:

| File | For |
|---|---|
| `linux-x86_64-cuda` | PC or server with an NVIDIA GPU |
| `linux-x86_64` | PC or server, CPU only |
| `linux-arm64-cuda` | ARM64 board with an NVIDIA GPU |
| `linux-arm64` | ARM64 board, CPU only |

The `-cuda` builds require the NVIDIA driver to be installed: they will not
start without it. If you have no GPU, take the CPU build.

```sh
tar -xzf i3k-rag-engine-v0.1.26-linux-x86_64-cuda.tar.gz
cd i3k-rag-engine-v0.1.26-linux-x86_64-cuda

cat > .env <<'EOF'
AUTH__JWT_SECRET=change-this-to-a-long-random-string
EOF

./i3k-rag-engine
```

On first run it downloads roughly **12 GB** of components and verifies each one
against a pinned sha256 — mostly the language and embedding models. Allow time
for it. Every later start is immediate.

When it is ready your browser opens at **http://localhost:8000**. The initial
admin password is printed in the log:

```
SAVE THIS PASSWORD — it will not be shown again!
```

Set `AUTH__ADMIN_DEFAULT_PASSWORD` in `.env` beforehand if you would rather
choose it yourself.

## What it does

**Documents.** PDF (including scanned pages, via OCR), DOCX, XLSX, HTML, TXT,
Markdown and CSV. Scanned PDFs are detected automatically — when a page yields
too little text it is rasterised and passed through Tesseract in Italian and
English.

**Retrieval.** Documents are split into overlapping chunks, embedded with
[BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3) (1024 dimensions, multilingual
across 100+ languages) and stored in Qdrant. A question is embedded the same way
and answered from the closest passages, with each source shown.

**Chat.** Conversations are kept, answers stream token by token, and history is
carried into follow-up questions.

**Users.** JWT authentication with three roles — user, super user, admin —
governing who may upload and delete.

**Backups.** Scheduled backups of both the SQLite database and the Qdrant
collection, so a restore brings back a consistent pair.

## Architecture

| | |
|---|---|
| **API and server** | Rust, [axum](https://github.com/tokio-rs/axum) |
| **Frontend** | React + Vite, compiled into the binary's directory |
| **Embeddings** | BAAI/bge-m3 through [Candle](https://github.com/huggingface/candle), in-process, GPU or CPU |
| **Vector database** | [Qdrant](https://qdrant.tech), 1024-dim, cosine distance |
| **Language model** | [eullm](https://github.com/eullm/eullm) as a separate process |
| **Application database** | SQLite through sqlx — users, documents, conversations |
| **OCR** | pdfium for rasterising, Tesseract through leptess |

The embedding model is loaded before the language model on purpose: eullm sizes
its own GPU offload from the free VRAM it observes at startup, so it has to see
the memory the embedding model has already taken.

## Configuration

Settings come from the environment or a `.env` file, using `__` to separate
levels. Exactly one has no default:

```sh
AUTH__JWT_SECRET=…            # required — signs the session tokens
```

Everything else is optional:

```sh
SERVER__PORT=8000
AUTH__ADMIN_DEFAULT_PASSWORD=…   # otherwise a random one is generated and logged
EULLM__MODEL=qwen3-14b           # only read when you run eullm yourself; when
                                 # the engine starts it, the GGUF path from
                                 # manifest.toml is used instead
QDRANT__COLLECTION=rag_documents
EMBEDDINGS__REQUIRE_GPU=false    # true = refuse to start without CUDA rather
                                 # than silently falling back to a much slower CPU
DATA__DIR=/path/to/data          # defaults to the binary's own directory
RUST_LOG=info
```

`EMBEDDINGS__REQUIRE_GPU` is worth knowing about: without it, a broken CUDA
setup degrades to CPU and ingestion goes from seconds to minutes, which is easy
not to notice. The fallback is always logged at error level and exposed on
`GET /api/info`.

## Measuring performance

The binary can benchmark itself on your own hardware and document:

```sh
./i3k-rag-engine --bench /path/to/document.pdf
```

It writes a Markdown report timing each stage of ingestion and inference —
extraction, chunking, embedding, upsert, prefill, decode — with charts showing
where the time goes. `--bench-live` instead records every real ingestion and
query of a session and reports on shutdown.

## Upgrading from 1.x

There is no automatic migration path, because v2 changes both the storage
layout and the runtime model. In practice: install v2 alongside the old system,
re-upload your documents, then decommission the Python stack once you are
satisfied.

The 1.x Python version remains available on the
[`python-legacy`](../../tree/python-legacy) branch and in the `1.x` tags. It is
no longer developed, but nothing has been deleted.

## Building from source

Most people do not need to: the release tarballs are self-contained. If you
want to modify the engine or target a platform we do not publish, see
[BUILD.md](BUILD.md) for the native dependencies and the feature flags.

```sh
cargo build --release --features ocr,cuda
```

## Privacy and security

- **No external calls.** After the first run, no network access is needed.
- **No telemetry.** Nothing is collected or sent, ever.
- **Local models.** Both the language model and the embeddings run on your
  hardware.
- **Verified components.** Every downloaded component is checked against a
  sha256 pinned in `manifest.toml`; a mismatch aborts the run.

Please report vulnerabilities privately rather than through a public issue.

## How to cite

```bibtex
@software{i3k_rag_engine,
  author    = {Marchetti, Francesco},
  title     = {i3k RAG Engine: self-hosted document intelligence},
  publisher = {i3k},
  url       = {https://github.com/I3K-IT/RAG-Enterprise}
}
```

The archived 1.x Python release has its own DOI:
[10.5281/zenodo.20413005](https://doi.org/10.5281/zenodo.20413005).

## Licence

Apache-2.0 — see [LICENSE](LICENSE). Third-party components, including the
models downloaded at runtime, keep their own licences: see
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

## Credits

Built on [Candle](https://github.com/huggingface/candle),
[Qdrant](https://qdrant.tech), [axum](https://github.com/tokio-rs/axum),
[eullm](https://github.com/eullm/eullm),
[BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3),
[Qwen](https://huggingface.co/Qwen), [Tesseract](https://github.com/tesseract-ocr/tesseract)
and [PDFium](https://pdfium.googlesource.com/pdfium/).

Made in the EU by [i3k](https://www.i3k.eu).
