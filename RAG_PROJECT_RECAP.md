# RAG ENTERPRISE - PROJECT STATUS & RECAP

## 🎯 CURRENT STATE

### ✅ WORKING
- **Setup.sh**: Fully automated installation (Docker, NVIDIA toolkit, PyTorch CUDA 12.8)
- **Backend**: FastAPI running, healthy, responds to queries
- **Qdrant**: Vector database operational, storing embeddings
- **Ollama**: Running with `mistral:latest` model loaded
- **Frontend**: Open WebUI responsive at http://localhost:3000
- **RAG Pipeline**: Works end-to-end from frontend
- **GPU**: CUDA 12.9 properly configured with PyTorch 2.x + cu128

### ⚠️ PARTIALLY WORKING
- **Tika OCR**: Installed (Apache Tika 3.2.3) but extraction issues
- **Document Sources**: Frontend shows 2 sources even when only 1 is relevant (accuracy issue)
- **API Upload**: Files uploaded via API aren't properly indexed

### ❌ ISSUES TO FIX
1. **Tika Text Extraction**: Returns 0 characters from .txt files via API (works with correct MIME type manually)
2. **Source Attribution**: Backend returns empty `"sources":[]` in API responses
3. **Frontend-API Mismatch**: Frontend works well, API doesn't index documents properly

---

## 🏗️ SYSTEM ARCHITECTURE

```
http://localhost:3000 (Frontend - Open WebUI)
         ↓
http://localhost:8000 (Backend - FastAPI)
         ↓
    ┌────┴────┬─────────┬──────────┐
    ↓         ↓         ↓          ↓
 Qdrant    Ollama   PaddleOCR   Tika
:6333      :11434   (in backend) :9998
```

### Container Names
- `rag-backend`: FastAPI backend + Embeddings (Sentence Transformers)
- `rag-frontend`: Open WebUI
- `rag-ollama`: Ollama LLM server (mistral:latest)
- `rag-qdrant`: Qdrant vector database

---

## 📊 PROFILE CONFIGURATIONS

### MINIMAL
- CPU: 8+
- RAM: 16GB
- GPU VRAM: 12-16GB
- LLM: mistral
- Embedding: all-mpnet-base-v2
- Time: ~15 min

### STANDARD (Current)
- CPU: 16+
- RAM: 32GB
- GPU VRAM: 24GB (RTX 5070 Ti: 12GB available)
- LLM: mistral (4.4GB)
- Embedding: all-MiniLM-L6-v2 (384 dims)
- Time: ~25 min

### ADVANCED
- CPU: 32+
- RAM: 128GB
- GPU VRAM: 48GB+
- LLM: llama2 (39GB)
- Embedding: instructor-large
- Time: ~60 min

---

## 🔧 RECENT FIXES APPLIED

### PyTorch CUDA Compatibility
- **Issue**: PyTorch compiled for CUDA 11.7, driver is 12.9
- **Fix**: Added `--index-url https://download.pytorch.org/whl/cu128` to Dockerfile
- **Current**: PyTorch 2.x with CUDA 12.8 support ✅

### Tika Configuration
- **Issue**: Tika endpoint returns 405/400 errors
- **Fix**: Changed from `POST /tika` with `files=` to `PUT /tika` with `data=` + proper `Content-Type` header
- **Status**: Partially working (manual curl works, API extraction still issues)

### NVIDIA Container Toolkit
- **Issue**: GPU not available to Docker
- **Fix**: Installed nvidia-container-toolkit, configured Docker daemon
- **Current**: Docker can access RTX 5070 Ti ✅

---

## 📁 PROJECT STRUCTURE

```
rag-enterprise-structure/
├── docker-compose.yml          # All services configuration
├── setup.sh                    # MAIN: Automated installation script
├── backend/
│   ├── Dockerfile             # Backend image with Tika + PyTorch
│   ├── requirements.txt        # Python dependencies
│   ├── app.py                 # FastAPI main app
│   ├── ocr_service.py         # Tika text extraction ⚠️
│   ├── embeddings_service.py  # Sentence Transformers
│   ├── qdrant_connector.py    # Vector DB interface
│   ├── rag_pipeline.py        # RAG logic
│   └── uploads/               # Uploaded documents
├── frontend/                   # Open WebUI config
└── uploads/                    # Persistent upload volume

Key Files to Check:
- backend/ocr_service.py: Tika extraction logic
- backend/rag_pipeline.py: Source attribution logic
- backend/app.py: API endpoints
```

---

## 🚀 SETUP.sh EXECUTION FLOW

```
[0/10] System Preparation
[1/10] Installing Docker
[2/10] Configuring Docker Permissions → REQUIRES RE-LOGIN
[3/10] Starting Docker Service
[4/10] Installing NVIDIA Container Toolkit
[5/10] Installing docker-compose
[6/10] Preparing Project & Models
[7/10] Pulling Docker Images
[8/10] Building Backend
[9/10] Starting Services
[10/10] Final Setup (Pull Ollama Model + Verification)
```

**Total Time**: ~60-90 minutes first run

---

## 🔴 CRITICAL ISSUES TO RESOLVE

### Issue 1: Tika Text Extraction (HIGH PRIORITY)
**Symptom**: `"✅ Estratti 0 caratteri"` in logs
```
ERROR:ocr_service:❌ Errore Tika: 405
```

**Root Cause**: Tika receives files but doesn't parse .txt correctly

**Current Code** (backend/ocr_service.py):
```python
def extract_text(self, file_path: str) -> str:
    with open(file_path, 'rb') as f:
        file_data = f.read()
    
    response = requests.put(
        self.TIKA_URL,
        data=file_data,
        headers={'Content-Type': 'text/plain'},  # Issue here?
        timeout=30
    )
```

**What Works Manually**:
```bash
curl -X PUT -H "Content-Type: text/plain" \
  --data-binary @/app/uploads/test_python.txt \
  http://localhost:9998/tika
# Returns: 200 OK with parsed text in XML
```

**Next Action**: Debug why manual curl works but Python requests doesn't

---

### Issue 2: Empty Sources in API Responses (HIGH PRIORITY)
**Symptom**: API query returns `"sources":[]` but frontend shows sources

**Expected**: 
```json
{
  "answer": "...",
  "sources": [
    {"file": "test_python.txt", "relevance": 0.87}
  ]
}
```

**Current**: 
```json
{
  "answer": "...",
  "sources": []
}
```

**Root Cause**: Qdrant finds 0 results when queried via API

**Why Frontend Works**: 
- Documents indexed from frontend caricamenti (loads) work
- But documents loaded via API don't get indexed

---

### Issue 3: Source Attribution Accuracy
**Symptom**: Frontend shows 2 sources when query only needs 1

**Example**:
- Query: "Quando è stato creato Python?"
- File 1: test_rag.txt (about Turing machines - irrelevant)
- File 2: test_python.txt (about Python - relevant)
- Shows: Both files as sources

**Root Cause**: Qdrant returns both, no filtering by relevance threshold

---

## 🧪 TESTING COMMANDS

### Check Backend Health
```bash
curl http://localhost:8000/health
# Should return: {"status":"healthy",...}
```

### Check Ollama
```bash
docker exec rag-ollama ollama list
# Should show: mistral:latest 4.4GB
```

### Check Qdrant
```bash
curl http://localhost:6333/health
# Should return: {"ok":true}
```

### Upload Document
```bash
curl -X POST http://localhost:8000/api/documents/upload \
  -F "file=@/tmp/test.txt"
# Watch logs: docker logs rag-backend -f
```

### Query RAG
```bash
curl -X POST http://localhost:8000/api/query \
  -H "Content-Type: application/json" \
  -d '{"query": "Your question?", "top_k": 5}'
# Check for "sources" field
```

### View Logs
```bash
docker logs rag-backend | grep -i "tika\|indexing\|sources"
```

---

## 📝 NEXT STEPS FOR NEW CHAT

1. **Fix Tika extraction** in ocr_service.py
   - Debug: Why Python requests fails when curl works?
   - Possible: Data encoding, headers, or request format

2. **Implement source attribution** in rag_pipeline.py
   - Extract document metadata from Qdrant results
   - Return sources in API response

3. **Add relevance filtering**
   - Only show sources with similarity > threshold (e.g., 0.7)
   - Prevent irrelevant documents in results

4. **Test end-to-end** via API
   - Upload document
   - Query RAG
   - Verify sources are shown

5. **Update setup.sh** with final tested configuration

---

## 🎯 SUCCESS CRITERIA

- ✅ Setup.sh installs everything automatically
- ✅ Backend extracts text from all file formats
- ✅ Documents indexed properly in Qdrant
- ✅ RAG queries return relevant sources
- ✅ API and Frontend both work identically
- ✅ No manual interventions needed after setup.sh

---

## 📋 HARDWARE SPECS (Testing Environment)

- **GPU**: NVIDIA RTX 5070 Ti (12GB VRAM) - Requires CUDA 12.8+
- **CUDA**: 12.9
- **CPU**: Multi-core processor
- **RAM**: 32GB+
- **OS**: Ubuntu 24.04
- **Docker**: 28.5.1
- **PyTorch**: 2.1.2 with CUDA 12.8

---

## 🔗 KEY FILES TO MODIFY IN NEXT CHAT

1. `backend/ocr_service.py` - Fix Tika extraction
2. `backend/rag_pipeline.py` - Add source attribution  
3. `backend/app.py` - Ensure sources passed to response
4. `setup.sh` - Final tweaks if needed
5. `docker-compose.yml` - Profile-based configurations

---

## ⚡ QUICK START AFTER FIX

```bash
cd ~/ai/rag-enterprise-complete/rag-enterprise-structure

# If starting fresh
./setup.sh standard

# If already running, just test
curl http://localhost:8000/health

# View all logs
docker compose logs -f rag-backend
```

---

**Last Updated**: November 3, 2025
**Status**: RAG functional from frontend, API issues being debugged
**Next Session Focus**: Fix Tika + Source Attribution
