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

---

## [0.1.35] - 2026-08-21

### Fixed

- **Ingestion crashed on real-world PDFs containing curly quotes, accents
  or em dashes.** `chunker::is_heading_line` (new in the heading-context
  feature above) sliced `line[..prefix.len()]` — a raw BYTE offset —
  which panics the instant a multi-byte character straddles that offset,
  taking the whole ingestion request down with it
  (`thread 'tokio-rt-worker' panicked ... is not a char boundary`).
  Reproduced on the real EU AI Act PDF within hours of shipping the
  heading-context feature. Rewritten to compare characters one at a time
  instead of byte-slicing — panic-free by construction regardless of
  content. New exhaustive test sweeps every prefix length × lead-in
  length combination with a multi-byte character landing at each
  possible offset, not just the one that happened to crash first.

- **Deleting a conversation with any messages in it always failed** with
  `FOREIGN KEY constraint failed (code 787)`. `chat_messages.conversation_id`
  has a (non-cascading) FK on `conversations(id)`
  (migrations/0002_conversations.sql) and sqlx enables
  `PRAGMA foreign_keys = ON` by default; `delete_conversation` deleted the
  parent row before its messages, which SQLite has always correctly
  refused. Fixed by deleting children before parent, within the same
  transaction. Both statements already filtered by `user_id` independently
  (not just `conv_id`), so reordering doesn't weaken the existing IDOR
  protection — see the updated doc comment on `delete_conversation`.

- **README quick-start couldn't actually be followed as written.** The
  release tarball is built as `tar czf ... -C stage .` — flat, no
  wrapping version-named folder inside it — but the quick-start snippet
  told users to `cd i3k-rag-engine-vX.Y.Z-linux-x86_64` right after
  `tar -xzf`, a directory that plain extraction never creates. Fixed to
  `mkdir` first. Also documented (new "Upgrading" section) that
  extracting a new release's tarball *over* an existing install reuses
  every already-downloaded component — `bootstrap::ensure_component`
  only fetches what's missing or sha256-mismatched, and `DATA__DIR`
  defaults to the binary's own directory — so this isn't a special
  workflow, just how the existing idempotent provisioning already
  behaves when pointed at a non-empty directory.

---

## [0.1.34] - 2026-08-21

### Added

- **Sources card now shows the page number(s) a chunk came from.** The
  API response has carried `page_start`/`page_end` since the Source
  Provenance Foundation work, but the frontend never rendered them —
  the sources list under an answer only showed filename + similarity
  %. `frontend/src/App.jsx` now adds a "pag. N" (or "pag. N–M" when a
  chunk spans multiple pages) badge next to each source, when the
  document has page info (non-PDF formats and documents ingested
  before this feature existed still have none, so the badge is
  omitted for those, not shown as blank/zero).

- **Chunks now carry the nearest preceding structural heading** ("Article
  99", "Chapter XII", "Section 4", ...) when they don't already contain
  it themselves. Root cause: chunk boundaries are byte-count-driven and
  know nothing about document structure, so a chunk can land entirely
  inside e.g. Article 100's body without the "Article 100" heading —
  which landed a chunk or two earlier. Retrieval then hands the LLM a
  fragment like "...administrative fines of up to EUR 1 500 000" with no
  indication of which article it belongs to. This is confirmed as the
  actual cause of two independently-built RAG stacks (this one and the
  old Python one) both misattributing the AI Act's Article 99/100/101
  penalty clauses to the wrong article on the same questions — verified
  against the real Official Journal PDF text, not assumed. New
  `chunker::detect_headings`/`inject_heading_context`: only touches the
  text that gets embedded/stored, not `Chunk.start_byte`/`end_byte` —
  page numbers and citation spans are unaffected. `CHUNKING_CONFIG_VERSION`
  bumped 1 → 2 (same chunk_index now stores different text than before,
  so re-ingesting must not collide with old provenance_ids).

---

## [0.1.33] - 2026-08-20

### Fixed (yet again)

- **`EULLM__MODEL_OVERRIDE` didn't stop the manifest-pinned qwen3-14b
  (8.4GB) from downloading anyway.** Same shape of bug as the bge-m3
  gguf one above: `select_components()` always selects "qwen3-14b" —
  a manifest model with no target, universal like every other model
  component — regardless of Settings, but `start_eullm` only ever
  falls back to it when `model_override` is unset. Once an override
  is set — a local path already on the machine, or a URL eullm
  fetches itself on `eullm run` — that download was never going to be
  read by anything. New `bootstrap::drop_unused_chat_model`, applied
  the same way as `drop_unused_embedding_model`, skips it whenever
  `EULLM__MODEL_OVERRIDE` is set, either form.

### Fixed (again)

- **bge-m3's GGUF was never going to resolve for eullm.** The manifest
  used to pin it as a component this binary downloaded directly via
  HTTP into eullm's model-store directory — but eullm's own model
  resolution (`resolve_model`) never downloads anything at request
  time; it only reads its store, mount points, or an explicit path
  (opt-in only). Removed `bge-m3-gguf` from manifest.toml entirely.
  It is now provisioned the correct way: `bootstrap::start_eullm`
  calls `eullm pull <url>` itself, once, the first time
  `ingestion_embedding=eullm` runs and the file isn't already
  present — idempotent and offline-safe like every other component,
  sha256-verified independently since a URL pull doesn't verify
  itself. Also fixes `config::EULLM_EMBEDDING_MODEL`, which was
  `"bge-m3"`: running a real pull against a real eullm 0.6.90 showed
  it actually registers as `"bge-m3-f16"` (derived from the pulled
  file's own name) — every `/api/embed` call under the old constant
  would have 404'd.

### Security

- **Qdrant no longer listens on all network interfaces.** The bundled
  qdrant binary defaults to `0.0.0.0` when its host isn't set —
  verified by running the actual pinned binary and reading its own
  startup log — and this project has no API key concept for it at
  all, so on any host where ports 6333/6334 weren't independently
  firewalled, the full REST+gRPC API (read, write, and delete every
  ingested document, completely unauthenticated) was reachable by
  anyone who could reach those ports, bypassing this binary's own JWT
  auth entirely. Now bound to `127.0.0.1` explicitly — this binary
  only ever talks to qdrant over localhost anyway, so nothing
  functional changes. Deployments with `DATA__MANAGE_SUBPROCESSES=false`
  (qdrant run externally) are unaffected either way — that qdrant's
  bind address was never this project's to control.
  eullm was checked too and needs no equivalent fix: it also listens
  on `0.0.0.0` at the socket level, but enforces its own IP allowlist
  at the application layer, loopback-only by default (`Allowed source
  IPs/subnets: 127.0.0.1/32, ::1/128`, per its own startup log) — a
  deliberate default, not an oversight.

### Fixed

- **Release tarballs did not include `.env.example`.** It existed in
  the repository since 0.1.32 but the packaging step never copied it
  into `stage/` alongside the binary, so anyone who downloaded a
  release instead of cloning the repo had no configuration template
  at all. Added to all three platform tarballs.

### Changed

- **`EMBEDDINGS__INGESTION_EMBEDDING=eullm` now also covers query-time
  embedding, not just ingestion.** Previously this mode only routed
  document-ingestion embedding through eullm's `POST /api/embed`;
  every question was still embedded through the in-process Candle
  instance regardless. Now both go through eullm, and Candle is not
  loaded at startup at all in this mode — not even on CPU — so
  bootstrap no longer downloads its ~2.1GB of bge-m3 weights either
  (only the ~1.1GB GGUF eullm itself uses). `Off` and `CandleGpu` are
  unaffected: query embedding still always uses Candle for those two,
  same as before. Worth knowing before enabling: on a card where
  bge-m3 and the chat model do not both fit in VRAM, every query now
  pays a potential model-swap round trip (evict chat to embed the
  question, evict bge-m3 back out to answer it) — see the doc comment
  on `config::IngestionEmbedding::Eullm`.

## [0.1.32] - 2026-08-20

### Fixed

- **`Settings::load()` rejected the minimal `.env` README.md has
  documented since 0.1.28.** Every field of `EullmSettings` already
  had its own default, but the `Settings.eullm` field itself had no
  `#[serde(default)]` and `EullmSettings` had no `Default` impl — with
  zero `EULLM__*` variables set, config loading failed outright with
  "missing field `eullm`" before ever reaching the individual fields'
  defaults. Verified against the actual compiled binary, not just the
  struct definitions: running it with only `AUTH__JWT_SECRET` set
  reproduced the failure before the fix, and reached the real
  bootstrap/download flow after it. Every other documented `.env`
  example (`BUILD.md`) happened to already set `EULLM__URL`/
  `EULLM__MODEL` explicitly, which is why this went unnoticed.

### Added

- `EULLM__REPEAT_LAST_N` — how many recent tokens `repeat_penalty`
  looks back over, previously hardcoded to 256 with no way to change
  it short of a rebuild. Default unchanged (256).
- `.env.example` at the repo root, documenting every `SECTION__FIELD`
  variable `Settings::load()` reads.

## [0.1.31] - 2026-08-20

### Added

- **Source Provenance Foundation.** Every retrieved chunk now carries a
  byte-offset span (`source_start`/`source_end`) into its source
  document, a PDF page span (`page_start`/`page_end`, for both the
  native `pdf_oxide` path and the OCR fallback), and a deterministic
  `provenance_id` anchored to the uploaded file's own sha256 — stable
  across re-ingestion of an unchanged file, and versioned so a later
  chunking/extraction config change produces visibly distinct ids
  instead of silent collisions. Infrastructural only: this is not
  claim-level (sentence) attribution, source highlighting, or
  NLI-based verification — those stay roadmap items. All new fields
  are optional; points written before this change deserialize them as
  absent, no re-ingestion required. See `rag::chunker::provenance_id`
  and `documents::parser::PageSpan`.

### Fixed

- **eullm never started on a GPU-less Linux x86_64 host.** The
  manifest's only `linux-x86_64` eullm pin required CUDA; a machine
  without an NVIDIA GPU found no usable target and bootstrap silently
  fell back to no LLM. Added the missing CPU-only entry (mirrors the
  ARM64 fix from 0.1.28).
- `--embedding-model` is now gated behind a separate
  `EullmSettings::reserve_embedding_model` flag (default off) instead
  of firing automatically whenever ingestion embedding runs through
  eullm — on a tight-VRAM card it was reserving space for bge-m3
  before `--fit` sized the chat model, starving it. Deployments with
  headroom for both can opt in explicitly.

### Changed

- CI no longer builds or publishes the Candle-CUDA Linux release
  variants (`release-linux-{x86_64,arm64}-cuda`): eullm's own bundled
  CUDA already covers GPU-accelerated embedding
  (`/api/embed` since 0.6.82, `--embedding-model` since 0.6.90), making
  a second CUDA toolchain compiled into this binary redundant. The
  source (`--features cuda`) stays buildable by hand; this only stops
  CI from shipping it. Every platform now ships one CPU-only tarball,
  matching how Windows was already described — a GPU is still used
  automatically wherever eullm detects one at runtime, regardless of
  how this binary itself was compiled.
- `manifest.toml` also pins the `eullm-linux-x64-vulkan` asset
  (sha256/size verified against the real downloaded binary). Not yet
  selectable — no Vulkan-GPU detection exists in `current_targets()`
  — pinned in advance so the entry is ready once that detection is
  written.

## [0.1.30] - 2026-08-18

### Added

- **Document-ingestion embedding through eullm.** `EMBEDDINGS__INGESTION_EMBEDDING=eullm`
  routes the embedding step of document ingestion through eullm's own
  `POST /api/embed` (eullm ≥ 0.6.82) instead of the in-process Candle path —
  an alternative for GPU-accelerated embedding that does not need this
  binary itself compiled with `--features cuda`, since eullm manages its own
  device placement. Query-time embedding is unaffected either way: it always
  runs through the resident Candle instance, a single short text per
  request. Off by default; existing deployments see no change unless this is
  set. See `config::IngestionEmbedding` for the two other explicit modes
  (`candle_gpu`, the pre-existing CPU↔GPU swap; `off`, unchanged).

  With eullm ≥ 0.6.90, `bootstrap::spawn_eullm` also passes
  `--embedding-model` at startup, so bge-m3 loads as a reserved companion
  next to the chat model when there is room for both — decided once, inside
  eullm, instead of gambled on which of two independently-started processes
  happened to claim VRAM first.

### Changed

- `manifest.toml`'s eullm fleet moves to 0.6.90 (all six pinned targets).

## [0.1.28] - 2026-08-17

### Added

- **Native Windows x86_64 support**, CPU and CUDA. `i3k-rag-engine.exe`
  cross-compiles from a Linux host (no Windows runner needed) — see
  BUILD.md's "Windows (cross-compiled from Linux, CPU only)". OCR
  (Tesseract 5.5.0 + Leptonica 1.85.0) is built statically for the release
  tarball, no runtime mingw dependency; the engine binary itself still needs
  `libstdc++-6.dll` bundled alongside it, a dependency Tesseract's own build
  does not have. Embedding is CPU-only on this platform (Candle's CUDA
  support is not cross-compiled for Windows); eullm — a separate process —
  still uses the GPU when one is present, so chat inference is accelerated
  regardless.
- OCR no longer needs a build-time `--features ocr` flag: `libtesseract` and
  `libleptonica` load at runtime through `libloading`, from an explicit path
  next to the executable, the same pattern already used for `libpdfium`. It
  is always compiled in now.

---

## [0.1.27] - 2026-08-14

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

  Verified end to end against a running Qdrant 1.18.2, not only in unit tests:
  seed a collection, back it up, add a point and a row, restore, and check that
  both additions are gone and the archived state is back. That test is
  `#[ignore]`d by default and runs with
  `QDRANT_URL_FOR_TEST=… cargo test qdrant_round_trip -- --ignored`.

- **Backups are verified, twice.** A backup nobody has checked is a guess, and
  the guess is only tested on the day it has to work.

  When the archive is written: the SQLite copy is reopened and put through
  `PRAGMA integrity_check`, and the Qdrant snapshot is checked against the size
  and sha256 Qdrant itself reports for it — a truncated download used to be
  written out silently. Each member's digest goes into a `backup.json` inside
  the archive.

  When it is restored: every member is checked against that manifest **before
  anything is written**. A damaged archive stops there, with the installation
  untouched, instead of being discovered halfway through.

  Two failures are now told apart. If Qdrant answers with a snapshot that does
  not match what it says it made, the backup fails outright — a verifiably
  broken archive must never reach the backup directory. If Qdrant does not
  answer at all, the archive is still written, without vectors, and says so:
  losing today's copy of the users and the document metadata as well would be
  worse, and the gap is recorded rather than hidden.

  Archives written by 0.1.25 and 0.1.26 carry no manifest. They still restore,
  and the response reports `verified: false` so it is clear they could not be
  checked.

### Fixed

- The Qdrant snapshot was downloaded and written with nothing verified at all.
  A truncated HTTP body is not an error — it just leaves a shorter file — so a
  half-downloaded snapshot was archived as though it were sound.
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

[Unreleased]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.27...HEAD
[0.1.27]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.26...v0.1.27
[0.1.26]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.25...v0.1.26
[0.1.25]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v0.1.25
[1.2.1]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v1.2.1
[1.2.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.2.0
[1.1.5]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.1.5
[1.1.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.1.0
[1.0.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.0.0
