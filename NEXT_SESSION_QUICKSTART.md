# 🎯 RAG ENTERPRISE - NEXT SESSION QUICK START

**Last Updated**: 2025-11-13  
**Current Status**: ⚠️ **NEEDS STABILIZATION TEST BEFORE COMMIT**

---

## 🚀 IMMEDIATE ACTIONS (Next Session)

### Step 1: Review Current State (5 min)
```bash
# Check if system is running
curl http://localhost:8000/health | jq '.status'
# Expected: "ok"

# Check Qdrant
curl http://localhost:6333/health | jq '.ok'
# Expected: true
```

### Step 2: Run Stabilization Test (30 min)

**Test Plan:**
```
Prepare 5 test documents:
1. CI_Franco2.pdf (improved ID card)
2. Technical document (TU-81 type)
3. Contract (if available)
4. Large PDF (>20MB)
5. Scanned document (low quality)

For EACH document:
  - Upload to system
  - Verify OCR extraction in logs
  - Run 3-5 specific queries
  - Check results accuracy
  - Note any hallucinations or errors

Track:
  ✅ Accuracy (correct answer returned?)
  ✅ Source attribution (correct document cited?)
  ✅ No hallucinations?
  ✅ Speed <10s?
```

### Step 3: Validate Threshold (10 min)

**Current Configuration:**
```yaml
RELEVANCE_THRESHOLD: 0.50  # (was 0.40)
```

**Test Query:**
```bash
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "Qual è il codice fiscale?",
    "top_k": 5
  }' | jq '.sources'

# Expected: 1-2 relevant documents
# NOT: 5 documents with mixed relevance
```

### Step 4: Fix Issues (if needed)

**If hallucinations still occur:**
```
❌ Problem: LLM invents data
   Fix 1: Check if threshold needs adjustment
   Fix 2: Validate field extraction with logs
   Fix 3: Improve LLM prompt

❌ Problem: Corrupted text in sources
   Fix: Add UTF-8 validation in qdrant_connector.py
   
❌ Problem: Wrong document cited
   Fix: Check retrieval scoring logic
```

### Step 5: Make First Commit

```bash
cd ~/ai/rag-enterprise-complete/

# If tests pass:
git add .
git commit -m "🎉 RAG Enterprise v1.0-beta: Multi-document support (threshold=0.50)

- Fixed retrieval with relevance threshold increase
- Tesseract fallback for scanned PDFs
- Field extraction for identity documents
- Conversational memory with context awareness
- Document type detection (4 types)
- Batch vector indexing (1000/batch)
- Custom React frontend
- GPU acceleration enabled"

git tag v1.0-beta
git log --oneline -5  # Verify
```

---

## 📊 TEST CHECKLIST

### Pre-Test Setup
- [ ] Read RAG_ENTERPRISE_STATUS_2025-11-13.md
- [ ] System running: `docker compose ps`
- [ ] All containers healthy
- [ ] 5 test documents prepared
- [ ] Backend logs readable: `docker logs rag-backend -f`

### During Test
- [ ] Upload each document and verify in logs
- [ ] For each document, run queries:
  - [ ] **Identity Card**: "Qual è il codice fiscale?" (if CI available)
  - [ ] **Any doc**: "Riassumi il documento in 3 punti"
  - [ ] **Multi-doc**: "Confronta le due versioni"
  - [ ] **Memory test**: Ask name → ask another question using "who is that person?"
  
- [ ] Check for:
  - [ ] ✅ Correct answer retrieved
  - [ ] ✅ Correct source cited
  - [ ] ✅ No hallucinations
  - [ ] ✅ Response time <10s
  - [ ] ✅ No encoding issues in sources

### Post-Test Summary
```
Results:
- Total queries tested: __
- Successful queries: __ (__)
- Hallucinations: __
- Wrong source attribution: __
- Encoding errors: __
- Average response time: __s

PASS/FAIL: ☐ PASS (ready for commit) / ☐ FAIL (needs fixes)
```

---

## 🔍 DEBUGGING TIPS

### Issue: Query returns wrong document

**Check logs:**
```bash
docker logs rag-backend -f | grep -A 5 "Searching for top"
# Look for: similarity scores of each document
# Should show: highest relevance doc FIRST
```

**Check threshold:**
```bash
docker inspect rag-backend | grep "RELEVANCE_THRESHOLD"
# Should show: 0.50
```

**If wrong:**
```bash
# Update docker-compose.yml:
# RELEVANCE_THRESHOLD: "0.55"  # (increase more)

sudo docker compose build backend
sudo docker compose restart backend
```

---

### Issue: LLM hallucinating data (e.g., fake CF)

**Check retrieval:**
```bash
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "codice fiscale", "top_k": 10}' | jq '.sources'

# If >3 documents with >0.40 similarity: THAT'S the problem
# Solution: increase threshold to 0.55-0.60
```

**Check field extraction:**
```bash
docker logs rag-backend -f | grep "Structured Fields"
# Should show extracted CF correctly

# If empty or wrong:
# - Document quality issue (OCR can't read)
# - Regex pattern doesn't match format
# - See: "Regex Field Extraction Too Fragile" in STATUS doc
```

---

### Issue: UTF-8 corruption in sources

**Check encoding:**
```bash
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "test"}' | jq '.sources[0].text' | head -c 100

# If shows: â¦â¦ or other corruption:
# Root cause: payload encoding issue in qdrant_connector.py
# Fix: Add UTF-8 validation
```

---

## 📋 FILES TO MONITOR

### Logs
```bash
# Real-time backend logs
docker logs rag-backend -f

# Filter by event type
docker logs rag-backend -f | grep "ERROR"      # Errors only
docker logs rag-backend -f | grep "WARNING"    # Warnings
docker logs rag-backend -f | grep "✅"         # Success
docker logs rag-backend -f | grep "TESTO"      # Text extraction
```

### Key Log Markers
```
✅ System starting = Good
✅ Tika ready at Xs = Good
✅ Model loaded = Good
✅ N embeddings generati = Good
✅ Inserted N vectors = Good
✅ Query completata = Good
⚠️ Extraction insufficient = May trigger fallback
❌ ERROR = Problem - investigate
```

---

## 🎯 SUCCESS CRITERIA

**Ready for Commit ONLY if:**
```
✅ 5+ documents tested
✅ 0 hallucinations in 20+ queries
✅ Correct source attribution in 100% of queries
✅ No encoding errors
✅ Response time always <10s
✅ Memory feature working (remembers context)
✅ All containers running without errors
```

**NOT Ready if:**
```
❌ Any hallucinated data
❌ Wrong document cited
❌ Text corruption in sources
❌ >1 query fails
❌ Any ERROR in logs
```

---

## 💡 WHAT TO DO AFTER COMMIT

Once v1.0-beta is committed:

### Week 1: Phase 1 (Stabilization)
- [ ] Implement Universal Document Schema System
  - [ ] Define schema for IDENTITY_CARD
  - [ ] Extract CF via LLM (not regex)
  - [ ] Validate CF format (16 chars)
  - [ ] Add fallback to manual entry
  
- [ ] Add reranker (bge-reranker-base)
  - [ ] Rerank top_k=5 results
  - [ ] Improve accuracy to 99%

### Week 2: Phase 2 (Features)
- [ ] Chat history persistence
- [ ] Document deletion
- [ ] Export results

### After: Roadmap
See "Future Roadmap" section in RAG_ENTERPRISE_STATUS_2025-11-13.md

---

## 🆘 IF SOMETHING BREAKS

**Nuclear Option (Full Reset):**
```bash
# Stop everything
sudo docker compose down

# Remove volumes (WARNING: deletes all data!)
sudo docker volume rm rag-*

# Remove images
sudo docker rmi rag-backend qdrant/qdrant ollama/ollama

# Start fresh
sudo docker compose up -d
sleep 30

# Check
curl http://localhost:8000/health
```

**Less Destructive Reset:**
```bash
# Keep data, just restart backend
sudo docker compose restart backend
sleep 10

# Test
curl http://localhost:8000/health
```

**Check What's Running:**
```bash
docker compose ps
# Should show 4 containers: qdrant, ollama, backend, frontend
# All with status "Up"
```

---

## 📞 QUICK COMMANDS

```bash
# Start system
cd ~/ai/rag-enterprise-complete/rag-enterprise-structure
sudo docker compose up -d

# Check status
curl http://localhost:8000/health | jq '.'

# View logs (real-time)
docker logs rag-backend -f

# Upload test document
curl -X POST http://localhost:8000/api/documents/upload \
  -F "file=@/path/to/test.pdf"

# Query
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "test query", "top_k": 3}'

# See indexed documents
curl http://localhost:8000/api/documents | jq '.'

# Stop system
sudo docker compose down

# Full restart
sudo docker compose down && sudo docker compose up -d && sleep 30
```

---

## 🎓 LEARNING PATH

If new to the project:

1. **Architecture** (10 min)
   - Read "Architettura Completa" section in STATUS doc
   - Look at diagram

2. **Components** (20 min)
   - Read each component description
   - Understand flow: OCR → Embed → Qdrant → RAG → API

3. **Current Issue** (5 min)
   - Read "Root Cause Analysis" section
   - Understand: threshold 0.40 too low → hallucinations

4. **How to Test** (30 min)
   - Follow test plan above
   - Run queries, check results

5. **How to Fix** (varies)
   - If tests fail, see "Debugging Tips"
   - Apply fixes from issues list

---

## 📌 KEY NUMBERS

```
Chunk size:              1000 chars
Chunk overlap:           100 chars
Embedding dimension:     1024 (BAAI/bge-m3)
Relevance threshold:     0.50 (CRITICAL - was 0.40)
LLM temperature:         0.7
Batch size:              1000 vectors
Query timeout:           600s
OCR timeout:             600s
Backend startup:         16s
Model load (first time): 2-3 min
Query response:          5-10s
```

---

## ✅ READY FOR NEXT SESSION!

**What to do:**
1. Read this file (2 min)
2. Read RAG_ENTERPRISE_STATUS_2025-11-13.md (10 min)
3. Run stabilization test (30 min)
4. Fix any issues (varies)
5. Make commit (5 min)
6. Report results to next chat

**Questions:** Check the STATUS document - it has everything explained!

Good luck! 🚀
