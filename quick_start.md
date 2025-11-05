# RAG ENTERPRISE - QUICK START GUIDE

## 📦 FILES TO DEPLOY

Replace these files in your project:

```
rag-enterprise-structure/
├── backend/
│   ├── app.py                 ← REPLACE (versione corretta)
│   ├── ocr_service.py         ← REPLACE (con MIME type + PaddleOCR)
│   ├── rag_pipeline.py        ← REPLACE (con filtering + dedup)
│   ├── embeddings_service.py  (unchanged)
│   ├── qdrant_connector.py    (unchanged)
│   └── requirements.txt        (unchanged)
├── docker-compose.yml          (unchanged)
├── setup.sh                    (unchanged)
└── frontend/                   (unchanged)
```

---

## ⚡ QUICKEST DEPLOYMENT (5 minutes)

### 1. Stop containers
```bash
cd ~/ai/rag-enterprise-complete/rag-enterprise-structure
docker compose down
```

### 2. Backup old files
```bash
cp backend/app.py backend/app.py.backup
cp backend/ocr_service.py backend/ocr_service.py.backup
cp backend/rag_pipeline.py backend/rag_pipeline.py.backup
```

### 3. Deploy new files
```bash
# Copy from outputs
cp /path/to/outputs/app.py backend/
cp /path/to/outputs/ocr_service.py backend/
cp /path/to/outputs/rag_pipeline.py backend/
```

### 4. Start containers
```bash
docker compose up -d
```

### 5. Verify
```bash
# Wait 10 seconds for startup
sleep 10

# Check health
curl http://localhost:8000/health | jq .

# Should see: "status": "healthy"
```

---

## 🧪 IMMEDIATE TEST (30 seconds)

```bash
# Test 1: Health
echo "1. Health check..."
curl -s http://localhost:8000/health | jq '.status'

# Test 2: Upload
echo -e "\n2. Upload test file..."
echo "Python è un linguaggio di programmazione" > /tmp/test.txt
curl -s -X POST http://localhost:8000/api/documents/upload \
  -F "file=@/tmp/test.txt" | jq '.document_id'

# Test 3: Wait for processing
echo -e "\n3. Waiting 3 seconds for background processing..."
sleep 3

# Test 4: Query
echo -e "\n4. Query..."
curl -s -X POST http://localhost:8000/api/query \
  -H "Content-Type: application/json" \
  -d '{"query": "Python", "top_k": 5}' | jq '{answer: .answer[0:50], sources: .sources, num_sources: .num_sources}'

# Expected output:
# {
#   "answer": "Python è un linguaggio di programmazione...",
#   "sources": [{"filename": "test.txt", "similarity_score": 0.95}],
#   "num_sources": 1
# }
```

---

## 📋 WHAT'S CHANGED (For Team)

### 🔧 ocr_service.py
- ✅ Dynamic MIME type detection
- ✅ Automatic PaddleOCR fallback
- ✅ Better error handling

### 🔧 rag_pipeline.py
- ✅ Relevance threshold filtering (default 0.5)
- ✅ Source deduplication
- ✅ Enhanced logging

### 🔧 app.py
- ✅ Detailed logging at each step
- ✅ Better error messages
- ✅ Improved response models
- ✅ Full stack traces on errors

---

## 🐛 TROUBLESHOOTING

### Issue: "sources": []

**Check 1: Is Qdrant receiving documents?**
```bash
docker logs rag-backend -f | grep "Indexing"
```

Should see:
```
✅ Indexing completato per 'test.txt'
```

**Check 2: Query finding documents?**
```bash
docker logs rag-backend -f | grep "Recuperati"
```

Should see:
```
✅ Recuperati N documenti
```

**Check 3: Above threshold?**
```bash
docker logs rag-backend -f | grep "sopra threshold"
```

If seeing "Nessun documento supra il threshold", lower it:

```bash
# Edit docker-compose.yml
RELEVANCE_THRESHOLD=0.3  # Lower from 0.5

# Restart
docker compose restart rag-backend
```

---

### Issue: "Estratti 0 caratteri"

**This means OCR failed**. Check logs:

```bash
docker logs rag-backend -f | grep -A5 "OCR Extraction"
```

Look for:
- ✅ "Estratti XXX caratteri con Tika" → Working
- ⚠️ "Tika ritornò testo vuoto, provando PaddleOCR..." → Fallback
- ✅ "Estratti XXX caratteri con PaddleOCR" → Fallback working

If both fail, check:
```bash
# Is Tika running?
docker ps | grep -i tika
docker logs rag-ollama  # Might be wrong container name

# Is Java available in backend?
docker exec rag-backend java -version
```

---

### Issue: Container won't start

**Check logs**:
```bash
docker logs rag-backend
```

**Common causes**:
- Python dependencies: `docker compose logs rag-backend | grep -i error`
- Port conflict: `lsof -i :8000`
- Out of memory: `docker stats`

**Solution**:
```bash
# Rebuild backend image
docker compose build --no-cache rag-backend
docker compose up -d
```

---

## 📊 MONITORING

### Real-time logs
```bash
docker logs rag-backend -f
```

### Tail last 50 lines
```bash
docker logs rag-backend | tail -50
```

### Filter by keyword
```bash
# Only errors
docker logs rag-backend | grep "❌"

# Only successes
docker logs rag-backend | grep "✅"

# Only queries
docker logs rag-backend | grep "Query"

# Only processing
docker logs rag-backend | grep "Processing"
```

---

## 🎯 ENVIRONMENT VARIABLES

Add to `docker-compose.yml` or `.env`:

```yaml
# Tuning
RELEVANCE_THRESHOLD=0.5      # 0.0-1.0, lower = more results
TOP_K=5                        # Number of docs to retrieve

# Performance
LLM_MODEL=mistral             # or llama2, neural-chat, etc.
EMBEDDING_MODEL=all-MiniLM-L6-v2  # or all-mpnet-base-v2

# Debugging
LOG_LEVEL=INFO                # or DEBUG for verbose
```

---

## ✅ VERIFICATION CHECKLIST

After deployment, run these to verify everything works:

- [ ] `curl http://localhost:8000/health` returns `"status": "healthy"`
- [ ] Upload a .txt file completes with 202 status
- [ ] Logs show `✅ Indexing completato`
- [ ] Query returns non-empty `sources` array
- [ ] `similarity_score` is between 0.0 and 1.0
- [ ] No duplicate sources in response
- [ ] Upload a .pdf file and verify OCR works
- [ ] Query finds results from uploaded files

---

## 🔄 ROLLBACK (If needed)

```bash
# Stop containers
docker compose down

# Restore backups
cp backend/app.py.backup backend/app.py
cp backend/ocr_service.py.backup backend/ocr_service.py
cp backend/rag_pipeline.py.backup backend/rag_pipeline.py

# Rebuild and restart
docker compose build --no-cache rag-backend
docker compose up -d

# Verify
curl http://localhost:8000/health
```

---

## 📞 SUPPORT

### If something breaks:
1. Check logs: `docker logs rag-backend | tail -100`
2. Look for error messages starting with ❌
3. Check if all containers are running: `docker ps`
4. Restart backend: `docker compose restart rag-backend`

### Common errors & solutions:

| Error | Solution |
|-------|----------|
| "Connection refused" on localhost | Check if containers are running: `docker ps` |
| "503 Services not initialized" | Wait 10 seconds, containers still starting |
| "0 caratteri estratti" | OCR failed, check file format is supported |
| "sources: []" | Lower RELEVANCE_THRESHOLD, or documents not indexed |
| "Cannot connect to Docker" | Restart Docker: `sudo systemctl restart docker` |

---

## 🚀 PRODUCTION DEPLOYMENT

Before going live:

### 1. Performance Testing
```bash
# Upload 10 files
for i in {1..10}; do
  echo "File content $i" > /tmp/test_$i.txt
  curl -s -X POST http://localhost:8000/api/documents/upload \
    -F "file=@/tmp/test_$i.txt" > /dev/null
done

# Query many times
for i in {1..5}; do
  time curl -s -X POST http://localhost:8000/api/query \
    -H "Content-Type: application/json" \
    -d '{"query": "test", "top_k": 5}' > /dev/null
done
```

### 2. Error Testing
```bash
# Upload invalid file
curl -X POST http://localhost:8000/api/documents/upload \
  -F "file=@/some/file.xyz"

# Bad query
curl -X POST http://localhost:8000/api/query \
  -H "Content-Type: application/json" \
  -d '{"query": ""}'
```

### 3. Load Testing
```bash
# Install hey (HTTP load tester)
go install github.com/rakyll/hey@latest

# Run load test
hey -c 10 -n 100 http://localhost:8000/health
```

---

## 📚 DOCUMENTATION

For detailed information:
- `TESTING_GUIDE.md` - Full testing procedures
- `TECHNICAL_DETAILS.md` - Deep dive into changes
- `RAG_PROJECT_RECAP.md` - Project history and status

---

**Version**: 1.0
**Date**: November 3, 2025
**Status**: ✅ Ready to Deploy