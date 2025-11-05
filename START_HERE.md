# 🎯 RAG Enterprise - START HERE

## Cosa Hai Ricevuto

Una **soluzione RAG Enterprise completa, production-ready** che comprende:

```
✅ Backend FastAPI + LangChain
✅ Frontend Open WebUI (ChatGPT-like)
✅ Vector Database Milvius
✅ OCR (PaddleOCR)
✅ Embeddings (Sentence-Transformers)
✅ LLM (Ollama - Mistral/Llama2)
✅ Docker Compose per deploy facile
✅ Setup automatizzati
✅ Documentazione completa
```

---

## 📂 File Importanti

1. **STRUCTURE_OVERVIEW.txt** ← LEGGI QUESTO PRIMO
   - Overview della struttura
   - Comandi principali
   - Quick start scenarios

2. **STACK_FINALE.md** ← Dettagli tecnici
   - Stack tecnologico scelto
   - Hardware requirements
   - Architettura dettagliata
   - Roadmap future

3. **rag-enterprise-structure/** ← Codice vero e proprio
   - Backend Python
   - Docker files
   - Setup scripts

4. **README.md** (dentro rag-enterprise-structure)
   - Documentazione architettura

5. **QUICKSTART.md** (dentro rag-enterprise-structure)
   - Guida rapida per diversi scenari

---

## 🚀 Come Iniziare (3 Step)

### Step 1: Scarica/Estrai
```bash
unzip rag-enterprise-structure.zip
cd rag-enterprise-structure
```

### Step 2: Esegui Setup
```bash
# Tutto insieme (consigliato per prima volta)
chmod +x setup.sh
./setup.sh

# Oppure se preferisci modulare:
make install-all
```

### Step 3: Accedi
```
Frontend: http://localhost:3000
Backend:  http://localhost:8000
Docs:     http://localhost:8000/docs
```

---

## 📋 Prerequisiti

**Hardware (Minimo):**
- CPU: i7/Ryzen 7 quad-core
- RAM: 32GB (64GB recommended)
- GPU: **RTX 5070 12GB** (critico per performance)
- SSD: 200GB+

**Software:**
- Docker (con NVIDIA Docker per GPU)
- Docker Compose v2+

**Sistema Operativo:**
- Linux (consigliato Ubuntu 22.04)
- macOS (con Colima/Rancher)
- Windows (WSL2 + Docker Desktop)

---

## 🎯 Scenari di Deployment

### 🏠 Scenario 1: Tutto su Una Macchina (Monolith)
```bash
./setup.sh
```
**Ideale per**: Prototipo, test, PMI

---

### 🏢 Scenario 2: Backend + Frontend Separati
```
Macchina 1 (GPU potente):  Backend + DB
Macchina 2 (CPU-only):     Frontend
```
**Ideale per**: Scalabilità frontend, separazione concerns

---

### 🌍 Scenario 3: Multi-Location (Enterprise)
```
Datacenter 1:  Milvius DB (shared)
Datacenter 2:  Backend
Office/Remote: Frontend
```
**Ideale per**: Enterprise, HA/DR, multi-sede

---

## 🔧 Comandi Utili

```bash
# Installation
make install-all          # Install everything
make install-db          # Database only
make install-backend     # Backend + Ollama
make install-frontend    # Frontend only

# Management
make logs                # Visualizza log
make stop                # Arresta
make restart             # Riavvia
make health              # Verifica salute

# Docker Compose direct
docker-compose up -d     # Avvia
docker-compose down      # Arresta
docker-compose ps        # Status
```

---

## 📊 Architettura

```
┌──────────────────────────────────────────┐
│  OPEN WEBUI (Frontend)                   │
│  http://localhost:3000                   │
└──────────────────┬───────────────────────┘
                   │ REST API
                   ↓
┌──────────────────────────────────────────┐
│  FASTAPI Backend                         │
│  http://localhost:8000                   │
│                                          │
│  - LangChain RAG                        │
│  - PaddleOCR                            │
│  - Embeddings (Sentence-Transformers)  │
│  - LLM (Ollama)                         │
└──────────────┬─────────────────────────┬─┘
               │                         │
           gRPC│                         │gRPC
               │                         │
               ↓                         ↓
┌─────────────────────────────────────────┐
│  MILVIUS Vector Database                │
│  TCP 19530 (gRPC), HTTP 9091           │
│                                         │
│  - Etcd (metadata)                      │
│  - MinIO (S3 storage)                   │
│  - Milvius Core (search)               │
└─────────────────────────────────────────┘
```

---

## ⚙️ Configurazione

Modifica `.env.example` per customizzare:

```bash
# Database
MILVIUS_HOST=milvius
MILVIUS_PORT=19530

# Backend
LLM_MODEL=mistral          # mistral, llama2, neural-chat, etc
EMBEDDING_MODEL=all-MiniLM-L6-v2
CUDA_VISIBLE_DEVICES=0     # GPU ID

# Frontend
BACKEND_URL=http://backend:8000
```

---

## 🧪 Test Post-Installazione

```bash
# 1. Verifica Backend
curl http://localhost:8000/health
# Deve ritornare: {"status":"healthy",...}

# 2. Accedi a Frontend
open http://localhost:3000

# 3. Upload documento di test
# Trascina un PDF

# 4. Poni una domanda
# "Qual è il contenuto principale di questo documento?"

# 5. Verifica risposta
# Deve avere fonte + risposta
```

---

## 📝 Workflow Tipico

```
1. Upload PDF/Documento
   ↓
   PaddleOCR estrae testo
   ↓
   Text splitter (chunks)
   ↓
   Sentence-Transformers genera embeddings
   ↓
   Milvius indicizza vettori

2. Query Utente
   ↓
   Query embedding (Sentence-Transformers)
   ↓
   Search in Milvius (top-k retrieval)
   ↓
   Build context da documenti
   ↓
   LLM genera risposta (Ollama)
   ↓
   Return answer + sources
```

---

## 🛠️ Performance Tuning

### Se è lento:

1. **OCR lento** → Dedica GPU: `CUDA_VISIBLE_DEVICES=0`
2. **Embedding lento** → Aumenta batch: `EMBEDDING_BATCH_SIZE=64`
3. **LLM lento** → Usa modello piccolo: `LLM_MODEL=neural-chat-7b`
4. **Sistema lento** → Aumenta RAM o riduci batch size

### Se VRAM is low:

```bash
# Quantize LLM (4-bit)
OLLAMA_LOAD_TIMEOUT=300s

# Reduce embedding batch
EMBEDDING_BATCH_SIZE=16
```

---

## 🔐 Sicurezza

✅ **Codice protetto** dentro container Docker
✅ **Network isolato** tra servizi
✅ **No source visible** nel deploy
✅ **Offuscamento disponibile** con PyArmor (su richiesta)
✅ **Backend non esposto** pubblicamente

---

## 📞 Support & Troubleshooting

### Backend non risponde?
```bash
docker-compose logs backend
docker-compose restart backend
```

### Milvius connection error?
```bash
# Verifica connection
telnet localhost 19530

# Restart Milvius
docker-compose restart milvius
```

### GPU non riconosciuta?
```bash
# Test GPU
docker run --rm --gpus all nvidia/cuda nvidia-smi

# Riavvia Docker daemon
sudo systemctl restart docker
```

### Out of Memory?
```bash
# Reduce batch sizes in .env
EMBEDDING_BATCH_SIZE=16
LLM_MODEL=neural-chat-7b
```

---

## 📚 Documentazione Completa

**Inside rag-enterprise-structure/**:

- `README.md` - Architettura dettagliata
- `QUICKSTART.md` - Setup per diversi scenari
- `backend/app.py` - API endpoints documentati
- `backend/Dockerfile` - Build configuration

---

## ✅ Checklist Pre-Produzione

- [ ] Hardware verificato (GPU, RAM, SSD)
- [ ] Docker + NVIDIA Docker installati
- [ ] Setup script eseguito senza errori
- [ ] Health check passato
- [ ] Test document upload completato
- [ ] Test query completato
- [ ] Performance accettabile
- [ ] Backup procedure documentata
- [ ] Firewall rules configurati
- [ ] Cliente briefed su funzionamento

---

## 🎉 Next Steps

1. **Leggi STRUCTURE_OVERVIEW.txt** per capire cosa c'è
2. **Esegui ./setup.sh** per installare
3. **Testa il sistema** con documento di prova
4. **Customizza .env** per tuoi settings
5. **Leggi STACK_FINALE.md** per dettagli tecnici
6. **Contattami** per domande/problemi

---

## 💡 Quick Tips

```bash
# Aggiorna modello LLM
docker exec rag-ollama ollama pull mistral

# Visualizza status servizi
docker-compose ps

# Backup Milvius data
docker-compose exec milvius tar -czf /tmp/backup.tar.gz /var/lib/milvius

# Pulisci cache
docker-compose exec backend rm -rf /app/uploads/*
docker system prune -a

# Mostra API docs
open http://localhost:8000/docs
```

---

## 🚀 Ready to Go!

Sei pronto per:
- ✅ Deploy locale
- ✅ Deploy distribuito
- ✅ Deploy multi-location
- ✅ Deploy enterprise con HA

**Buona fortuna! 🎉**

---

**Version**: 1.0.0 RAG Enterprise  
**Date**: October 2025  
**Support**: Internal documentation  

