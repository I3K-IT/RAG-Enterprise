# Roadmap

This document tracks the evolution of the project. Each version builds on the previous one, progressively improving retrieval quality, user experience, and system reliability.

All features listed here are planned for this open-source project. Contributions and feedback are welcome — open an issue or submit a PR!

---

## v1.0 — Foundation (Current)

- [x] One-command setup with Docker Compose
- [x] Multi-format document processing (PDF, DOCX, PPTX, XLSX, TXT, MD, ODT, RTF, HTML, XML)
- [x] Local LLM inference via Ollama (Qwen2.5, Mistral 7B Q4)
- [x] Vector search with Qdrant and BAAI/bge-m3 embeddings
- [x] OCR pipeline for scanned documents (Tesseract + Apache Tika)
- [x] JWT authentication with role-based access control (User / Super User / Admin)
- [x] Conversational memory per user
- [x] GPU acceleration support (NVIDIA CUDA)
- [x] 29-language support for document processing

---

## v1.1 — UI & Quality

Focus: modernize the chat interface and improve out-of-the-box answer quality.

- [ ] **Modern Chat UI** — Vertical message layout with user/assistant messages stacked in a familiar conversational format (similar to ChatGPT-style interfaces)
- [ ] **Copy Button** — One-click button to copy assistant responses to clipboard
- [ ] **Smart Chunking** — Increase chunk size to 2000 characters with 400 overlap (up from 1000/100) to preserve more surrounding context per chunk and reduce mid-sentence splits
- [ ] **Footnote Handling** — Instruct the LLM prompt to ignore OCR artifacts from footnotes (e.g. superscript numbers that look like decimals mid-sentence)
- [ ] **Query-Language Response** — The system responds in the same language the user writes the question in, regardless of the document language

---

## v1.2 — Precision & Theme

Focus: major retrieval accuracy boost via re-ranking, plus UI polish.

- [ ] **Cross-Encoder Re-ranking** — Add BAAI/bge-reranker-base (multilingual) as a second-stage ranker after vector retrieval. This is the single most impactful improvement for answer quality, expected to increase accuracy by 25–40%
- [ ] **Light/Dark Theme Toggle** — Allow users to switch between light and dark themes, with the preference persisted in `localStorage`
- [ ] **Pipeline Progress Indicator** — Show the user each step of the RAG pipeline as it executes (Embedding → Search → Re-ranking → LLM Generation) instead of a generic loading spinner
- [ ] **Offline Mode** — Full offline operation after the initial model download, enabled via `HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1` environment variables

---

## v1.3 — Search & Persistence

Focus: hybrid retrieval, persistent history, and smarter resource allocation.

- [ ] **Hybrid Search (BM25 + Vector)** — Add BM25 keyword search alongside vector search, with Reciprocal Rank Fusion (RRF) to merge results. BM25 catches exact keyword matches that semantic search may miss; vector search captures meaning. Together they cover more ground
- [ ] **Persistent Chat History** — Store chat messages in SQLite with per-user storage (up to 100 messages), automatically restored on login or page refresh
- [ ] **GPU Memory Optimization** — Move embedding model and re-ranker to CPU, freeing GPU VRAM entirely for the LLM (Ollama). This significantly speeds up inference by eliminating GPU memory contention
- [ ] **BM25 Auto-Rebuild** — Automatically rebuild the BM25 index on backend startup to ensure keyword search stays in sync with the vector store

---

## v1.4 — Intelligence

Focus: smarter retrieval through metadata awareness and relevance filtering.

- [ ] **Self-Querying Metadata Filter** — Automatically detect document names mentioned in user queries and perform targeted per-document retrieval instead of searching the entire collection
- [ ] **Document ID Cache** — In-memory cache of document IDs to avoid scanning all Qdrant points on every query, reducing retrieval latency
- [ ] **Improved Relevance Filter** — Apply a re-ranker score threshold (> 0.52) to exclude low-relevance results before they reach the LLM, reducing hallucinations from noisy context

---

## Version History

| Version | Codename                | Status      |
|---------|-------------------------|-------------|
| v1.0    | Foundation              | Released    |
| v1.1    | UI & Quality            | Planned     |
| v1.2    | Precision & Theme       | Planned     |
| v1.3    | Search & Persistence    | Planned     |
| v1.4    | Intelligence            | Planned     |

---

## Contributing

Want to help build the next version? Check the [Contributing Guide](CONTRIBUTING.md).

Priorities may shift based on community feedback. Open an issue to suggest features, report bugs, or vote on what matters most to you.
