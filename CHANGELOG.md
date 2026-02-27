# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [1.1.0] - 2026-02-27

### Added

- **Backup & Restore system** with full admin panel UI
  - One-click local backup of database, documents, and vector store
  - Cloud backup support via rclone (70+ providers: Mega, S3, Google Drive, OneDrive, Dropbox, WebDAV, FTP, SFTP, B2, pCloud)
  - Automatic scheduled backups with cron expressions and configurable retention policies
  - Selective restore (choose which components to restore individually)
  - Cloud provider management with connection testing
  - Backup history tracking (last 100 operations)
  - Download backups from cloud to local storage
- Complete backup documentation ([docs/BACKUP.md](docs/BACKUP.md)) with setup guides for all providers
- rclone pre-installed in Docker image for cloud storage integration

### Security

- All backup endpoints require admin role authentication
- Cloud provider passwords encrypted via rclone obscure mechanism
- Path traversal protection on archive extraction during restore
- Safe online SQLite backup (no downtime, no data corruption)

---

## [1.0.0] - 2026-02-21

First public release of RAG Enterprise — a 100% local Retrieval-Augmented Generation system for businesses that need complete data privacy.

### Added

- One-command setup with Docker Compose (`setup.sh`)
- Multi-format document processing (PDF, DOCX, PPTX, XLSX, TXT, MD, ODT, RTF, HTML, XML)
- Local LLM inference via Ollama (Qwen3 14B Q4, Mistral 7B Q4)
- Vector search with Qdrant and BAAI/bge-m3 multilingual embeddings
- OCR pipeline for scanned documents (Tesseract + Apache Tika)
- JWT authentication with role-based access control (User / Super User / Admin)
- Conversational memory per user with session isolation
- GPU acceleration support (NVIDIA CUDA)
- 29-language support for document processing and retrieval
- React + Vite frontend with Tailwind CSS
- Auto-configure network and security during setup
- Smart PDF detection and routing (digital vs scanned)
- Benchmark script for performance testing
- Community files: Contributing guide, issue templates, PR template, roadmap
- Qdrant API key support for secured deployments

### Security

- Production-ready JWT + CORS configuration
- Conversation isolation between users
- Removal of all hardcoded credentials
- Automatic security configuration during setup

### Performance

- Direct Ollama API client replacing LangChain wrapper
- Optimized RAG search parameters for large documents
- GPU memory management with automatic CPU fallback
- Tika heap tuning (4GB) with auto-restart on failure
- Robust timeout and auto-recovery for document processing
- Thread pool execution for document processing to prevent event loop blocking
- OOM crash prevention on sequential document uploads

### Fixed

- PyTorch and CUDA compatibility across GPU generations
- Embedding batch size tuning with CUDA fallback
- HTTP enforcement for local Qdrant connections
- Benchmark script authentication and output paths
- PaddleOCR / PyMuPDF dependency conflict resolution

### Changed

- LLM switched from Qwen 2.5 to Qwen3 14B Q4_K_M for improved quality
- All Italian text translated to English for international accessibility

[1.1.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v1.1.0
[1.0.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v1.0.0
