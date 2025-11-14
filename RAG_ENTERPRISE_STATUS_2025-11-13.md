# 🚀 RAG ENTERPRISE - COMPLETE STATUS REPORT
**Generated**: 2025-11-13  
**Version**: 1.0 (Pre-Commit)  
**Status**: ⚠️ **FUNCTIONAL BUT NEEDS STABILIZATION**

---

## 📋 EXECUTIVE SUMMARY

**RAG Enterprise** è un **sistema RAG enterprise-grade self-contained** per il deployment autonomo in aziende. È operazionale con architecture completa, ma **necessita stabilizzazione del retrieval** prima del primo commit.

### ✅ Cosa Funziona
- Setup automatico (setup.sh)
- OCR end-to-end (Tika + Tesseract fallback)
- Embedding con BAAI/bge-m3 (multilingue, 1024-dim)
- Vector storage in Qdrant (batch insertion)
- LLM generation con Ollama (neural-chat)
- Memory conversazionale
- Document type detection (IDENTITY_CARD, GENERIC_DOCUMENT, etc.)
- Structured field extraction (CF, indirizzo, data da CI)
- React custom frontend
- GPU acceleration (NVIDIA CUDA)

### ❌ Cosa Needs Fixing
- Retrieval retrieva documenti sbagliati con >3 documenti
- Threshold 0.40 troppo basso → contamina risposte
- LLM hallucina dati (CF fake) quando retrieval confuso
- Testo corrotto in alcuni sources (encoding UTF-8)
- No validation che i dati estratti siano corretti

### 🎯 KPI Attuale
- ✅ Single document: CF corretto
- ❌ Multiple documents: CF inventato
- ✅ Speed: 2-5 secondi per query
- ✅ OCR quality: Tesseract estrae bene da PDF di qualità
- ❌ Stabilità: Instabile con >3 documenti

---

## 🏗️ ARCHITETTURA COMPLETA

```
┌─────────────────────────────────────────────────────────────┐
│                    REACT FRONTEND (3000)                     │
│  - Upload File/Directory                                     │
│  - Query with Chat History                                   │
│  - Display Results + Sources with Download Links             │
│  - Real-time Processing Status                               │
└────────────────────┬────────────────────────────────────────┘
                     │ HTTP/WebSocket
┌────────────────────▼────────────────────────────────────────┐
│                   FASTAPI BACKEND (8000)                     │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ app.py - Main Orchestrator                           │   │
│  │  - POST /api/documents/upload                        │   │
│  │  - POST /api/query                                   │   │
│  │  - GET /api/documents                                │   │
│  │  - GET /api/documents/{id}/download                  │   │
│  │  - GET /health                                       │   │
│  └──────────────────────────────────────────────────────┘   │
│                          ↓                                    │
│  ┌─────────────┬──────────────┬──────────────┬────────────┐ │
│  │   OCR       │  EMBEDDINGS  │ RAG PIPELINE │  QDRANT    │ │
│  │ Service     │  Service     │              │ Connector  │ │
│  └─────────────┴──────────────┴──────────────┴────────────┘ │
└────────────────────┬────────────────────────────────────────┘
                     │ (localhost/docker network)
        ┌────────────┼────────────┬──────────────┐
        ▼            ▼            ▼              ▼
    ┌────────┐  ┌──────────┐  ┌────────┐  ┌─────────┐
    │ TIKA   │  │ TESSERACT│  │ OLLAMA │  │ QDRANT  │
    │ 9998   │  │ (in img) │  │ 11434  │  │ 6333    │
    │        │  │          │  │        │  │         │
    └────────┘  └──────────┘  └────────┘  └─────────┘
     PDF→Text    Fallback      LLM        Vector DB
     OCR         OCR           (neural-   (Cosine
                               chat:7b)   Search)
```

---

## 📁 PROJECT STRUCTURE

```
rag-enterprise-complete/
├── rag-enterprise-structure/          # Main RAG system
│   ├── backend/
│   │   ├── app.py                     # FastAPI main app (441 lines)
│   │   ├── rag_pipeline.py            # RAG core logic (450+ lines)
│   │   ├── ocr_service.py             # OCR + Tesseract (280+ lines)
│   │   ├── embeddings_service.py      # Sentence-Transformers (200+ lines)
│   │   ├── qdrant_connector.py        # Vector DB ops (300+ lines)
│   │   ├── Dockerfile                 # Backend image (OCR + CUDA)
│   │   ├── requirements.txt           # Python dependencies
│   │   └── uploads/                   # Document storage
│   ├── docker-compose.yml             # Orchestration (backend + deps)
│   ├── setup.sh                       # Automated installation
│   └── QUICKSTART.md                  # Quick reference
│
└── frontend/                          # Custom React Frontend (EXTERNAL)
    ├── Dockerfile.frontend
    ├── src/
    │   ├── App.jsx                    # Main React component
    │   ├── index.jsx
    │   └── App.css
    └── public/
```

---

## 🔧 CORE COMPONENTS DEEP DIVE

### 1️⃣ **OCR SERVICE** (`ocr_service.py`)

**Purpose**: Extract text from any document format

**Flow**:
```
PDF/DOCX/TXT
    ↓
Tika (primary OCR)
    ↓ (if <100 chars extracted)
Tesseract Fallback (PDF→Images→OCR)
    ↓
Clean XML/Text
    ↓
Return raw text
```

**Key Features**:
- ✅ Supports 15+ file formats (PDF, DOCX, PPT, XLS, etc.)
- ✅ Tika server (localhost:9998) with aggressive process management
- ✅ Tesseract fallback for scanned PDFs
- ✅ Automatic MIME type detection
- ✅ 600-second timeout for large files

**Current Issues**:
- ❌ Tika returns only metadata XML for image-based PDFs
- ⚠️ Tesseract fallback triggers only if <100 chars (should be 500+)
- ⚠️ No logging of actual extracted text (hard to debug)

---

### 2️⃣ **EMBEDDINGS SERVICE** (`embeddings_service.py`)

**Purpose**: Generate dense vector representations of text

**Model Stack**:
| Model | Dimension | Language | Use Case |
|-------|-----------|----------|----------|
| BAAI/bge-m3 | 1024 | Multilingual | **CURRENT** - SOTA dense+sparse |
| bge-large-en | 1024 | English | Alternative for EN |
| e5-large-v2 | 1024 | Multilingual | Alternative |
| all-MiniLM-L6-v2 | 384 | Multilingual | Lightweight |

**Current Config**: `BAAI/bge-m3` (1024-dim, multilingue, MIT license)

**Performance**:
- ✅ GPU-accelerated (CUDA)
- ✅ Batch processing (32-item batches)
- ✅ Normalized embeddings (cosine similarity)
- ✅ Fast: ~30ms per document

---

### 3️⃣ **QDRANT CONNECTOR** (`qdrant_connector.py`)

**Purpose**: Vector database operations (insert, search, delete)

**Key Features**:
- ✅ Batch insertion (1000 vectors/batch)
- ✅ Cosine distance metric
- ✅ Payload metadata storage (filename, document_id, text, etc.)
- ✅ Automatic collection creation
- ✅ Connection pooling with 600s timeout

**Configuration**:
```yaml
Collection: "rag_documents"
Vector Size: 1024 (matches embeddings)
Distance: Cosine
Batch Size: 1000
```

**Current Issues**:
- ❌ Payload sometimes not returned in searches
- ⚠️ No pagination for large result sets
- ⚠️ No duplicate detection/prevention

---

### 4️⃣ **RAG PIPELINE** (`rag_pipeline.py`)

**Purpose**: Orchestrate retrieval + LLM generation + source attribution

**Flow**:
```
User Query
    ↓
Embed Query (BAAI/bge-m3)
    ↓
Search Qdrant (top_k=5, threshold=0.50)
    ↓
Filter by Relevance (score ≥ 0.50)
    ↓
Build Context from Retrieved Chunks
    ↓
Format Prompt with History + Context + Question
    ↓
LLM Generation (Ollama neural-chat:7b)
    ↓
Extract Sources (deduplicate by doc_id)
    ↓
Return (answer_text, sources_list)
```

**Key Features**:
- ✅ Memory-aware prompting (considers conversation history)
- ✅ Relevance threshold filtering (default 0.50)
- ✅ Chunk-based retrieval with metadata
- ✅ Source deduplication (keeps highest similarity score)
- ✅ Smart context building

**Chunking Strategy**:
```python
chunk_size=1000 chars
overlap=100 chars
separator=["\n\n", "\n", ".", " ", ""]
```

**LLM Config**:
```
Model: neural-chat:7b (via Ollama)
Temperature: 0.7
Context Window: 4096 tokens
Base URL: http://ollama:11434
```

**Current Issues**:
- ❌ LLM hallucinátes when retrieval ambiguous (invents CF)
- ❌ Threshold 0.40 too low → passes irrelevant docs
- ⚠️ No validation that extracted data is correct
- ⚠️ No retry mechanism for failed queries

---

### 5️⃣ **FASTAPI BACKEND** (`app.py`)

**Purpose**: HTTP API + document processing orchestration

**Endpoints**:

```
POST /api/documents/upload
├─ Input: file (PDF, DOCX, etc.)
├─ Process: OCR → Chunking → Embedding → Indexing
└─ Output: {document_id, status, processing_time}

POST /api/query
├─ Input: query, top_k, temperature, history
├─ Process: Embed → Search → Generate → Format
└─ Output: {answer, sources, processing_time}

GET /api/documents
├─ Input: none
└─ Output: list of uploaded documents

GET /api/documents/{doc_id}/download
├─ Input: document_id
└─ Output: binary PDF file

GET /health
├─ Input: none
└─ Output: {status, services_status, uptime}
```

**Document Type Detection** (`detect_document_type()`):
```python
if 'CARTA DI IDENTITA' in text: return 'IDENTITY_CARD'
elif 'PASSAPORTO' in text: return 'PASSPORT'
elif 'PATENTE DI GUIDA' in text: return 'DRIVING_LICENSE'
else: return 'GENERIC_DOCUMENT'
```

**Field Extraction** (Regex-based):
```
IDENTITY_CARD:
├─ codice_fiscale: [A-Z]{6}\d{2}[A-Z]\d{2}[A-Z]\d{3}[A-Z]
├─ numero_carta: [A-Z]{2}\d{6}[A-Z]{2}
└─ indirizzo: VIA/VIALE + numero + città

PASSPORT, DRIVING_LICENSE: [Not yet implemented]
```

**Background Processing**:
- ✅ Async document processing (doesn't block API)
- ✅ Logging to console + rotating files
- ✅ Error handling + recovery

**Current Issues**:
- ❌ Field extraction fragile (regex breaks with formatting changes)
- ⚠️ No input validation on queries
- ⚠️ No rate limiting
- ⚠️ No authentication

---

## 📊 CONFIGURATION

### Docker Compose (`docker-compose.yml`)

```yaml
Services:
├─ qdrant (6333)           # Vector database
│  └─ Volume: qdrant-data (persistent)
├─ ollama (11434)          # LLM server
│  ├─ Volume: ollama-data
│  └─ GPU: CUDA device 0
├─ backend (8000)          # FastAPI
│  ├─ Env: RELEVANCE_THRESHOLD=0.50
│  ├─ Volume: uploads, huggingface-cache
│  └─ GPU: CUDA device 0
└─ frontend (3000)         # React UI
   └─ Env: REACT_APP_API_URL=http://localhost:8000

Restart Policy: unless-stopped
Network: rag-network (bridge)
GPU Support: NVIDIA Container Toolkit required
```

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| QDRANT_HOST | qdrant | Vector DB host |
| QDRANT_PORT | 6333 | Vector DB port |
| LLM_MODEL | neural-chat | LLM model name |
| EMBEDDING_MODEL | BAAI/bge-m3 | Embedding model |
| RELEVANCE_THRESHOLD | 0.50 | Min relevance score |
| CUDA_VISIBLE_DEVICES | 0 | GPU device ID |

---

## ✅ WORKING FEATURES

### Document Processing
- ✅ Single file upload
- ✅ Batch directory upload
- ✅ 15+ file formats (PDF, DOCX, PPT, XLS, TXT, etc.)
- ✅ OCR extraction (text + scanned)
- ✅ Automatic chunking (1000-char chunks)
- ✅ Embedding generation (BAAI/bge-m3)
- ✅ Batch vector indexing (1000/batch)
- ✅ Persistent storage (Docker volumes)

### Querying
- ✅ Natural language queries
- ✅ Conversational memory (5 last exchanges)
- ✅ Relevance-based filtering (threshold=0.50)
- ✅ Source attribution with similarity scores
- ✅ Document download from results
- ✅ GPU acceleration

### Document Understanding
- ✅ Document type detection (4 types)
- ✅ Structured field extraction (CF, address, date)
- ✅ Smart memory (remembers previous subjects)
- ✅ Named entity awareness

### Infrastructure
- ✅ Docker containerization (4 containers)
- ✅ Automated setup (setup.sh)
- ✅ GPU support (NVIDIA CUDA 12.8+)
- ✅ Health checks (/health endpoint)
- ✅ Persistent volumes
- ✅ Network isolation

---

## ❌ KNOWN ISSUES & LIMITATIONS

### 🔴 CRITICAL

**Issue 1: Retrieval Ambiguity with Multiple Documents**
```
Symptom: Query about Francesco Marchetti → finds wrong document
Status:  With 1 document: ✅ Works. With >3: ❌ Fails
Root Cause: Threshold 0.40 passes too many documents → LLM confused
Solution:  Increase threshold to 0.55-0.60
Timeline:  Needs testing with 5+ documents
```

**Issue 2: LLM Hallucination**
```
Symptom: Invents data (CF: "051969FRM78D102F" when should be "MRCFNC69E20E329H")
Status:  Triggered by ambiguous retrieval context
Root Cause: LLM generates plausible-looking but fake data when unsure
Solution:  1) Fix retrieval, 2) Add validation, 3) Better prompting
```

### 🟡 HIGH PRIORITY

**Issue 3: Corrupted Text in Sources**
```
Symptom: Some sources show: "â¦â¦â¦" instead of readable text
Root Cause: UTF-8 encoding issue in payload handling
Solution:  Explicit UTF-8 validation in qdrant_connector.py
```

**Issue 4: Regex Field Extraction Too Fragile**
```
Symptom: CF not extracted if formatting changes slightly
Root Cause: Hardcoded regex patterns don't account for OCR variations
Solution:  Implement universal Document Schema system (see Roadmap)
```

### 🟠 MEDIUM PRIORITY

**Issue 5: No Validation**
```
Symptom: System accepts and indexes any extracted data
Root Cause: No schema validation
Solution:  Add validation layer (16-char CF, valid date format, etc.)
```

**Issue 6: No Rate Limiting**
```
Symptom: Can spam API with requests
Solution:  Add FastAPI rate limiting middleware
```

**Issue 7: No Authentication**
```
Symptom: API is open to anyone
Solution:  Add API key auth or OAuth2
```

---

## 🚦 TEST RESULTS (2025-11-13)

### Single Document Test ✅
```
File: CI_Franco2.pdf (improved quality)
Upload: 2.94s
├─ OCR: 1.34s → 903 chars
├─ Chunking: 0.00s → 1 chunk
└─ Indexing: 1.60s

Query: "Qual è il codice fiscale?"
Response Time: 3.5s
Result: ✅ MRCFNC69E20E329H (CORRECT!)
Sources: CI_Franco2.pdf (54.9%)
```

### Multiple Documents Test ❌
```
Files: CI_Franco.pdf, CI_Franco2.pdf, TU-81.pdf, monjero.pdf, [technical docs]
Queries Tested: 5
Results:
├─ Name + Birth: ✅ Correct (memory helps)
├─ Address: ⚠️ Found right doc but cited wrong one (49% vs 54%)
├─ CF (1st attempt): ❌ Hallucinated "051969FRM78D102F"
├─ CF (after restart): ✅ Correct for 1 doc, then ❌ after adding more
└─ Overall: INSTABLE - depends on document order and memory state
```

### Performance Benchmarks
| Operation | Time | Status |
|-----------|------|--------|
| Backend startup | 16s | ✅ Good |
| Model load (first query) | 2-3min | ⚠️ Slow |
| Subsequent queries | 5-10s | ✅ Good |
| OCR (text PDF) | 1-2s | ✅ Good |
| OCR (scanned 16MB) | 30-60s | ⚠️ Acceptable |
| Embedding (1K chunks) | 45s | ✅ Good |
| Indexing (batch 1000) | 30s | ✅ Good |

---

## 🎯 CURRENT PROBLEM ANALYSIS

### Root Cause: Threshold Too Low

**Scenario** (with 5 documents):
```
Query: "codice fiscale di marchetti"

Qdrant Results:
├─ CI_Franco.pdf: 0.56 ✅ CORRECT
├─ TU-81.pdf: 0.45 ❌ WRONG
├─ monjero.pdf: 0.42 ❌ WRONG
├─ technical.pdf: 0.41 ❌ WRONG
└─ other.pdf: 0.39 🚫 Below current threshold

With threshold=0.40:
Context passed to LLM: [correct doc] + [3 wrong docs]
LLM sees conflicting information → HALLUCINATION

With threshold=0.50:
Context passed to LLM: [correct doc] ONLY
LLM sees clear information → CORRECT ANSWER
```

### Why This Happens
1. Semantic similarity isn't perfect (partial matches score high)
2. Multiple documents contain similar keywords
3. LLM can't reliably distinguish when multiple sources conflict
4. No ranking/reranking in retrieval pipeline

### Quick Fix vs Proper Fix
```
QUICK FIX (today):  threshold: 0.40 → 0.50
Expected Result: Resolves ~80% of hallucinations

PROPER FIX (future):
├─ Add reranker (bge-reranker)
├─ Implement BM25 hybrid search
├─ Add document-level filtering
└─ Improve LLM prompting
```

---

## 🛣️ IMMEDIATE NEXT STEPS (Before Commit)

### 1️⃣ Stabilization Testing (TODAY)
```bash
# Test with 5 documents of different types
- Upload 5 diverse PDFs
- Run 10+ queries on each
- Log all results
- Verify: no hallucinations, correct source attribution
```

### 2️⃣ Fix Identified Issues (TODAY)
```
[ ] Increase threshold to 0.55
[ ] Add UTF-8 validation in qdrant_connector.py
[ ] Log actual extracted text in ocr_service.py
[ ] Test 50+ queries across all documents
```

### 3️⃣ Documentation (TODAY)
```
[ ] Update README with current limitations
[ ] Document known issues
[ ] Add troubleshooting guide
```

### 4️⃣ First Commit (AFTER TESTS PASS)
```bash
git add .
git commit -m "Initial RAG Enterprise 1.0 - Single/multi-doc support (threshold=0.55)"
git tag v1.0-beta
```

---

## 🚀 FUTURE ROADMAP

### Phase 1: STABILIZATION (1-2 weeks)
- [ ] Universal Document Schema System
  - Define schema for each doc type
  - Extract via LLM (not regex)
  - Validate results
  - Fallback to user confirmation
- [ ] Add reranker (bge-reranker-base)
- [ ] Implement BM25 hybrid search
- [ ] Add rate limiting + auth

### Phase 2: FEATURES (2-3 weeks)
- [ ] Multi-language support (>50 languages)
- [ ] Chat history persistence
- [ ] Document deletion
- [ ] Document versioning
- [ ] Export results (PDF, CSV, JSON)

### Phase 3: ADVANCED (4-8 weeks)
- [ ] Role-based access control
- [ ] Document classification (internal/external/confidential)
- [ ] Multi-database (one per category)
- [ ] Voice assistant (Whisper + TTS)
- [ ] LLM fine-tuning with company data
- [ ] Advanced analytics/monitoring

### Phase 4: PRODUCTION (8+ weeks)
- [ ] Multi-user support
- [ ] Enterprise auth (OAuth2, SAML)
- [ ] Encryption at rest
- [ ] Audit logging
- [ ] SLA monitoring
- [ ] High availability setup
- [ ] Load balancing

---

## 📚 HOW TO USE THIS FOR NEXT CHAT

**Context to Provide:**

```markdown
Read this document first: RAG_ENTERPRISE_STATUS_2025-11-13.md

Current state:
- ✅ Single document: Working perfectly
- ❌ Multiple documents: Retrieval ambiguous (threshold issue)
- ⚠️ Needs: Stabilization test with 5+ documents

Next session focus:
1. Run comprehensive test suite
2. Fix identified issues
3. Make first commit
4. Then proceed to Phase 1 (Universal Schema System)
```

**Quick Onboarding:**
1. Understand architecture (see diagram above)
2. Read component descriptions (OCR → Embeddings → Qdrant → RAG → API)
3. Review test results and known issues
4. Pick tasks from "Immediate Next Steps"

---

## 📞 QUICK REFERENCE

### File Locations
```
Backend Core:  ~/ai/rag-enterprise-complete/rag-enterprise-structure/backend/
Config:        ~/ai/rag-enterprise-complete/rag-enterprise-structure/docker-compose.yml
Frontend:      ~/ai/rag-enterprise-complete/frontend/
Setup:         ~/ai/rag-enterprise-complete/rag-enterprise-structure/setup.sh
```

### Key Commands
```bash
# Check status
curl http://localhost:8000/health | jq '.'

# View logs
docker logs rag-backend -f | grep -E "ERROR|WARNING|âœ…"

# Restart services
sudo docker compose restart backend

# Full reset
sudo docker compose down
rm -rf ~/ai/rag-enterprise-complete/rag-enterprise-structure/backend/uploads/*
sudo docker compose up -d

# Test query
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "test", "top_k": 3}'
```

### Important Thresholds
```
RELEVANCE_THRESHOLD: 0.50 (was 0.40, increased for stability)
CHUNK_SIZE: 1000 chars (balanced)
CHUNK_OVERLAP: 100 chars (for context)
BATCH_SIZE: 1000 vectors (performance)
TEMPERATURE: 0.7 (balanced creativity/accuracy)
```

---

## 📝 VERSION HISTORY

| Version | Date | Status | Changes |
|---------|------|--------|---------|
| 1.0-beta | 2025-11-13 | In Progress | Initial architecture, single-doc working, multi-doc needs fix |
| 0.9 | 2025-11-07 | Previous | Batch insertion working, OCR improved |
| 0.8 | Earlier | Archive | Initial setup |

---

## ✍️ NOTES FOR NEXT CHAT

1. **Current Blocker**: Threshold ambiguity with multiple documents
   - Quick fix ready (increase to 0.55)
   - Need validation with 5+ documents
   - After fix: ready for commit

2. **Biggest Technical Debt**: Regex-based field extraction
   - Works for one doc type at a time
   - Breaks with formatting variations
   - Future: Replace with Universal Schema System

3. **Performance is Good**: Not the bottleneck
   - Query time: 5-10s (acceptable)
   - OCR time: 1-30s (depends on file size)
   - Indexing: scales well with batch insertion

4. **What's Missing for Production**:
   - Validation layer
   - Error recovery
   - Rate limiting
   - Authentication
   - Monitoring/alerting
   - Documentation

---

**Ready for next session!** 🚀

Questions or clarifications: check Immediate Next Steps section above.
