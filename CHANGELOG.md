# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

> **On the version numbers.** `1.0.0` to `1.2.1` are the Python stack,
> preserved on the [`python-legacy`](../../tree/python-legacy) branch and no
> longer developed. `0.1.x` is the Rust rewrite that replaced it: a new codebase,
> which restarts the numbering at `0.1` because its storage layout and HTTP
> surface are not settled yet. So the newer line carries the *lower* number —
> see [Upgrading from 1.x](README.md#upgrading-from-1x).

---

## [Unreleased]

### Added

- **Restore.** `POST /api/admin/backup/restore` puts an archive back, as the
  exact inverse of the backup that produced it: the Qdrant snapshot is uploaded
  with `priority=snapshot` so the archived vectors win, and every table the
  archive shares with the live schema is replaced inside one transaction. Until
  now a backup could be taken and listed but never used, which made the whole
  feature ornamental.

  Details worth knowing before running it:

  - It **replaces**, it does not merge. Rows created after the backup are gone.
  - Qdrant is restored first. If that fails nothing else is touched, so a failed
    restore leaves the installation as it was rather than stranding fresh
    metadata against old vectors.
  - An archive older than the current schema still restores: tables that no
    longer exist are skipped, and columns added by a later migration keep their
    default.
  - `_sqlx_migrations` is never copied back — the schema belongs to the binary
    that is running, not to the archive.
  - The response reports what was actually restored, because an archive taken
    while Qdrant was unreachable contains no snapshot.

### Fixed

- `backup/mod.rs` described the archive as including an "optional rclone
  upload". There is no rclone anywhere in this codebase and never was; backups
  are local files and nothing is uploaded anywhere.

---

## [0.1.26] - 2026-08-13

### Changed

- **The first run downloads 5 GB less** — `qwen3-8b` was pinned in
  `manifest.toml` but referenced nowhere in the code, left over from a pipeline
  this engine does not have. The first run goes from ~17.3 GB to ~12.3 GB. Every
  component is still verified against a pinned sha256 before use. An existing
  installation can delete `models/qwen3-8b/` to reclaim the space.
- `EULLM__MODEL` now defaults to `qwen3-14b` and is no longer required. Starting
  without it used to fail, for a setting the default configuration then ignores:
  when the engine launches eullm itself it passes the GGUF path from the
  manifest. It still applies if you run eullm separately and point the engine at
  it. `AUTH__JWT_SECRET` is now the only setting with no default.
- Remaining Italian text translated to English: the 503 returned while a document
  is being ingested, the errors reported when the first run cannot download or
  verify a component, and the stage labels of the `--bench` report.
- README: documents what actually changed between the `1.x` Python stack and
  this one, and states that the two cannot run side by side — they contend for
  ports 8000, 6333 and 11434, so the Compose stack must be stopped first. The
  `1.x` volumes are left in place, so going back remains possible.

### Fixed

- `Cargo.toml` declared `MIT OR Apache-2.0`; this project is Apache-2.0, as
  `LICENSE` has always said.
- The README still advertised a 17 GB first-run download after the manifest had
  changed, and still listed `EULLM__MODEL` as required after it had been given a
  default.

### Removed

- The `license/` module — dead code (`check_page_limit` was never called) that
  described a commercial gating model not implemented here, and pulled in
  `ed25519-dalek` for nothing.
- Source comments referencing files and internal documents absent from this
  repository, some of which described unreleased plans. Three stale `TODO`
  markers on code that has been complete and working for a long time.

### Notes

No changes to retrieval, ingestion, chunking, embeddings or the HTTP API.
Upgrading is a matter of replacing the binary; data and Qdrant collections are
untouched.

---

## [0.1.25] - 2026-08-13

First public release of the Rust rewrite. The system is now a **single binary**
with no Docker, no Compose and no Java: on first run it downloads and
sha256-verifies everything it needs — vector database, inference engine, models,
OCR data — and supervises those processes itself. After that it needs no network
at all.

### Added

- Single self-contained binary for Linux x86\_64 and arm64, with and without
  CUDA, each published as a release tarball.
- Component bootstrap driven by `manifest.toml`: every download is pinned by
  sha256 and size, and a mismatch aborts the run. Platform variants are selected
  at runtime, including a CIX P1 build of the inference engine gated on actual
  CPU feature detection rather than on the SoC name.
- Embeddings in-process via [Candle](https://github.com/huggingface/candle)
  (BAAI/bge-m3, 1024 dimensions), on GPU or CPU, with
  `EMBEDDINGS__REQUIRE_GPU` to refuse to start rather than silently degrade to a
  much slower CPU path.
- LLM inference through [eullm](https://github.com/eullm/eullm) as a supervised
  subprocess, with VRAM handed back and forth around ingestion.
- Document ingestion for PDF (including scanned pages via OCR), DOCX, XLSX, HTML,
  TXT, Markdown and CSV. Scanned pages are detected automatically and passed
  through Tesseract in Italian and English.
- JWT authentication with three roles — user, super user, admin.
- Streaming answers over SSE, with conversation history carried into follow-ups.
- Scheduled backups of the SQLite database and the Qdrant collection together, so
  a restore brings back a consistent pair.
- `--bench` writes a Markdown report timing each stage of ingestion and inference
  on your own hardware and document; `--bench-live` records a whole session
  instead and reports on shutdown.

### Changed

- Apache-2.0, with a full third-party inventory in
  [THIRD\_PARTY\_LICENSES.md](THIRD_PARTY_LICENSES.md).
- The Python stack is preserved unchanged on
  [`python-legacy`](../../tree/python-legacy) and in the `1.x` tags. There is no
  automatic migration path: stop the Compose stack, re-upload the documents, then
  decommission it once satisfied.

---

## [1.2.1] - 2026-05-27

### Added

- `.zenodo.json` with project metadata, keywords, ORCID-linked author and
  licence, enabling automatic DOI generation via [Zenodo](https://zenodo.org/)
  so the project is citable as a research output. ORCID iD:
  [0009-0003-8613-3065](https://orcid.org/0009-0003-8613-3065).

No functional changes from 1.2.0.

---

## [1.2.0] - 2026-03-02

### Added

- **Multi-GPU support** — NVIDIA (CUDA), AMD (ROCm) and CPU-only modes
  ([#9](https://github.com/I3K-IT/RAG-Enterprise/issues/9))
  - New `GPU_TYPE` setting in `.env` (`nvidia`, `amd`, `cpu`)
  - Docker Compose override files: `docker-compose.nvidia.yml` (NVIDIA CUDA),
    `docker-compose.amd.yml` (AMD ROCm)
  - Setup wizard now asks GPU type and auto-configures the correct Docker images
    and device mappings
  - AMD uses the `ollama/ollama:rocm` image with `/dev/kfd` and `/dev/dri`
    device passthrough
  - CPU-only mode works out of the box with no GPU drivers required

---

## [1.1.5] - 2026-03-01

### Added

- **Automatic model download at startup** — if the configured LLM model is not
  present in Ollama, the backend downloads it showing real-time progress:
  percentage, downloaded/total size, speed and estimated time remaining
- **Ollama readiness check** — the backend waits for Ollama to be reachable
  before proceeding, preventing 404/connection errors on fresh installations

### Fixed

- **Ollama URL now configurable** via `OLLAMA_HOST` and `OLLAMA_PORT` environment
  variables — previously hardcoded to `http://ollama:11434`, which only worked
  inside Docker networking
- Replaced stale `MILVIUS_HOST`/`MILVIUS_PORT` env vars in the Dockerfile with
  correct `OLLAMA_HOST`/`OLLAMA_PORT` defaults

---

## [1.1.0] - 2026-02-27

### Added

- **Backup & Restore system** with full admin panel UI
  - One-click local backup of database, documents and vector store
  - Cloud backup via rclone (70+ providers: Mega, S3, Google Drive, OneDrive,
    Dropbox, WebDAV, FTP, SFTP, B2, pCloud)
  - Automatic scheduled backups with cron expressions and configurable retention
    policies
  - Selective restore (choose which components to restore individually)
  - Cloud provider management with connection testing
  - Backup history tracking (last 100 operations)
  - Download backups from cloud to local storage
- Complete backup documentation (`docs/BACKUP.md`) with setup guides for all
  providers
- rclone pre-installed in the Docker image for cloud storage integration

### Security

- All backup endpoints require admin role authentication
- Cloud provider passwords encrypted via rclone obscure mechanism
- Path traversal protection on archive extraction during restore
- Safe online SQLite backup (no downtime, no data corruption)

---

## [1.0.0] - 2026-02-21

First public release of RAG Enterprise — a 100% local Retrieval-Augmented
Generation system for organisations that need complete data privacy.

### Added

- One-command setup with Docker Compose (`setup.sh`)
- Multi-format document processing (PDF, DOCX, PPTX, XLSX, TXT, MD, ODT, RTF,
  HTML, XML)
- Local LLM inference via Ollama (Qwen3 14B Q4, Mistral 7B Q4)
- Vector search with Qdrant and BAAI/bge-m3 multilingual embeddings
- OCR pipeline for scanned documents (Tesseract + Apache Tika)
- JWT authentication with role-based access control (user / super user / admin)
- Conversational memory per user with session isolation
- GPU acceleration support (NVIDIA CUDA)
- 29-language support for document processing and retrieval
- React + Vite frontend with Tailwind CSS
- Auto-configuration of network and security during setup
- Smart PDF detection and routing (digital vs scanned)
- Benchmark script for performance testing
- Community files: contributing guide, issue templates, PR template, roadmap
- Qdrant API key support for secured deployments

### Security

- Production-ready JWT + CORS configuration
- Conversation isolation between users
- Removal of all hardcoded credentials
- Automatic security configuration during setup

### Performance

- Direct Ollama API client replacing the LangChain wrapper
- Optimised RAG search parameters for large documents
- GPU memory management with automatic CPU fallback
- Tika heap tuning (4 GB) with auto-restart on failure
- Robust timeout and auto-recovery for document processing
- Thread pool execution for document processing, to avoid blocking the event loop
- OOM crash prevention on sequential document uploads

### Fixed

- PyTorch and CUDA compatibility across GPU generations
- Embedding batch size tuning with CUDA fallback
- HTTP enforcement for local Qdrant connections
- Benchmark script authentication and output paths
- PaddleOCR / PyMuPDF dependency conflict resolution

### Changed

- LLM switched from Qwen 2.5 to Qwen3 14B Q4\_K\_M for improved quality
- All Italian text translated to English for international accessibility

---

[Unreleased]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.26...HEAD
[0.1.26]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.25...v0.1.26
[0.1.25]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v0.1.25
[1.2.1]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v1.2.1
[1.2.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.2.0
[1.1.5]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.1.5
[1.1.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.1.0
[1.0.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.0.0
