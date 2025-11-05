# RAG Enterprise - Stack Finale Scelto

## 📋 Riepilogo Scelte Tecnologiche

### OCR
- **Scelta**: **PaddleOCR**
- **Motivo**: Open-source, multilingue, GPU-accelerated, ottimo per documenti sporchi/tabelle
- **Vantaggio**: Niente dipendenza da servizi terzi

### Embedding
- **Scelta**: **Sentence-Transformers** (modello: `all-MiniLM-L6-v2`)
- **Motivo**: Open-source, supporto Hugging Face, comunità grande, indipendenza da DeepSeek
- **Vantaggio**: 22MB, veloce, 384-dim, perfetto per enterprise

### Vector Database
- **Scelta**: **Milvius** (standalone con Docker)
- **Motivo**: Robusto, open-source, self-hosted, scalabile, con etcd + MinIO per HA
- **Vantaggio**: Pieno controllo, nessun lock-in cloud

### LLM
- **Scelta**: **Llama 2 13B** (o Mistral 7B per velocità)
- **Motivo**: Open-source, community support, runs localmente su RTX 5070 (12GB VRAM)
- **Runtime**: Ollama (semplice, container, GPU support)

### Backend
- **Framework**: **FastAPI** (veloce, async, auto-docs)
- **Orchestrazione RAG**: **LangChain**
- **Linguaggio**: Python 3.10

### Frontend
- **Scelta**: **Open WebUI** (clone ChatGPT/Claude open-source)
- **Motivo**: Interfaccia bellissima, multimodal-ready, nessuno sviluppo custom

### Deployment
- **Containerizzazione**: **Docker + Docker Compose**
- **Protettione IP**: **PyArmor** (offuscamento codice Python)
- **Networking**: Docker networks (isolamento, semplicità)

---

## 🏗️ Architettura Hardware Supportata

### Macchina Minima
```
CPU: i7/Ryzen 7 (quad-core)
RAM: 32GB
GPU: RTX 5070 12GB (richiesta per OCR + embedding + LLM)
Storage: 200GB SSD (modelli + dati)
```

### Macchina Ottimale
```
CPU: i9/Ryzen 9 (8+ core)
RAM: 64GB
GPU: RTX 5070 12GB + RTX 4080 16GB (multi-GPU)
Storage: 500GB+ SSD
```

---

## 🚀 Architettura Software

```
┌──────────────────────────────────────────────────────────┐
│                    CLIENT LAYER                          │
│                   (Open WebUI)                           │
│                   http://localhost:3000                  │
└────────────────────────┬─────────────────────────────────┘
                         │ REST API
                         ↓
┌──────────────────────────────────────────────────────────┐
│                  BACKEND LAYER (FastAPI)                 │
│                   http://localhost:8000                  │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │   PaddleOCR  │  │ Sentence-T   │  │   LangChain  │  │
│  │              │  │  Transformers│  │   + Ollama   │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└────────┬────────────────────────────────────────────┬───┘
         │                                            │
    gRPC │                                            │ gRPC
         │                                            │
         ↓                                            ↓
┌──────────────────────────────────────────────────────────┐
│                    STORAGE LAYER                         │
│                                                          │
│  ┌─────────────────────────────────────────────────┐   │
│  │           MILVIUS Vector Database               │   │
│  │      (TCP 19530 - gRPC, HTTP 9091)            │   │
│  │                                                 │   │
│  │  ├─ Etcd (metadata, distributed state)        │   │
│  │  ├─ MinIO (S3-compatible object storage)       │   │
│  │  └─ Milvius Core (vector search)              │   │
│  └─────────────────────────────────────────────────┘   │
│                                                          │
│  Volumi Persistenti:                                    │
│  ├─ /var/lib/milvius (vector data)                     │
│  ├─ /minio_data (backup/chunks)                        │
│  └─ /etcd (metadata)                                   │
└──────────────────────────────────────────────────────────┘
```

---

## 📊 Flusso Dati Completo

### 1. Upload Documento
```
User → Browser (3000) 
  → Backend API (8000)
    → PaddleOCR (estrae testo)
    → Text Splitter (chunking)
    → Sentence-Transformers (genera embeddings)
    → Milvius (indexing)
```

**Tempo**: ~5-30 sec per documento (dipende da size)

### 2. Query
```
User → Frontend (3000)
  → Backend API (8000)
    → Sentence-Transformers (query embedding)
    → Milvius (vector search)
    → LangChain RAG (context building)
    → Ollama/LLM (generate answer)
  → Frontend (risposta visualizzata)
```

**Tempo**: ~2-5 sec per query

---

## 🔧 Configurazione Distribuzione

### Opzione 1: Tutto su una macchina (Monolith)
- ✅ Semplice setup
- ✅ Performance non compromessa
- ❌ Single point of failure

```bash
./setup.sh
```

### Opzione 2: Backend + Frontend Separati
- ✅ Frontend scalabile (CPU-only)
- ✅ Backend su macchina potente
- ✅ DB condiviso

```
Macchina 1: Backend (RTX 5070) + Milvius
Macchina 2: Frontend (CPU-only)
```

### Opzione 3: Tutti Separati (Datacenter Ready)
- ✅ Massima scalabilità
- ✅ HA/DR possibile
- ✅ Multi-location

```
Datacenter 1: Milvius (shared DB)
Datacenter 2: Backend 1, Backend 2 (load balanced)
Office/Remote: Frontend (indipendente)
```

---

## 🛡️ Sicurezza & Protezione IP

### Code Obfuscation
```bash
# Backend code offuscato con PyArmor
pyarmor gen --restrict backend/*.py

# Docker image non contiene code leggibile
docker build -t rag-backend .
```

### Accesso Remoto
```dockerfile
# Backend NON esposto pubblicamente
# Frontend esposto via reverse proxy + auth
```

### Variabili Ambiente
```bash
# Sensibili in .env (non in git)
# Secrets gestiti via Docker secrets (Swarm) o Vault
```

---

## 📈 Roadmap & Estensioni Future

### Phase 2 (Q1 2026)
- [ ] Kubernetes deployment ready
- [ ] Monitoring + Prometheus metrics
- [ ] WebSocket per streaming risposte
- [ ] Multi-tenant architecture

### Phase 3 (Q2 2026)
- [ ] GraphQL API
- [ ] Document versioning + audit trail
- [ ] Fine-tuning UI per custom models
- [ ] Analytics dashboard

### Phase 4 (Q3 2026)
- [ ] Multi-language support UI
- [ ] Advanced security (RBAC, encryption)
- [ ] Backup automation
- [ ] Performance optimization (index caching)

---

## 📋 Checklist Pre-Produzione

### Hardware
- [ ] Verifica RTX 5070 + drivers NVIDIA
- [ ] Verifica 32-64GB RAM
- [ ] Verifica 200GB+ SSD available
- [ ] Test network latency Milvius

### Software
- [ ] Docker installato + NVIDIA Docker
- [ ] Docker Compose v2+
- [ ] Firewall rules (19530, 8000, 3000)

### Configurazione
- [ ] .env configurato correttamente
- [ ] Milvius connection test: `telnet host 19530`
- [ ] Backend health check: `curl http://backend:8000/health`
- [ ] Frontend raggiungibile e responsive

### Data
- [ ] Test upload documento di prova
- [ ] Test query su documento
- [ ] Verify embedding storage in Milvius
- [ ] Test backup procedure

### Deployment
- [ ] Documentazione scritta per cliente
- [ ] Runbook per troubleshooting
- [ ] Support contact info
- [ ] Escalation procedure

---

## 📞 Supporto Tecnico

### Logs
```bash
docker-compose logs -f                    # Tutti i servizi
docker-compose logs -f backend            # Solo backend
docker-compose logs -f milvius            # Solo DB
```

### Health Checks
```bash
curl http://localhost:8000/health         # Backend health
curl http://localhost:9091/healthz        # Milvius health
curl http://localhost:3000                # Frontend UP
```

### Common Issues

| Problema | Causa | Soluzione |
|----------|-------|-----------|
| Backend non si connette a Milvius | Network | Verifica firewall, MILVIUS_HOST in .env |
| GPU non riconosciuta | NVIDIA Docker mancante | Installa nvidia-docker, restart daemon |
| Modelli non caricano | Connessione internet lenta | Attendi download, ~5GB per modelli |
| Frontend lento | Frontend CPU-bound | Muovi su macchina dedicata se possibile |
| Milvius crashes | OOM | Aumenta host RAM o riduci batch size |

---

## 💡 Performance Tips

### Per OCR veloce
```bash
# Usa GPU per OCR
CUDA_VISIBLE_DEVICES=0  # Dedica GPU 0 solo a OCR
```

### Per Embedding veloci
```bash
# Aumenta batch size se VRAM disponibile
EMBEDDING_BATCH_SIZE=64  # Default 32
```

### Per LLM veloce
```bash
# Usa modello più piccolo ma veloce
LLM_MODEL=neural-chat     # 7B, velocissimo
LLM_MODEL=mistral         # 7B, balanced
# Evita
LLM_MODEL=llama2-large    # 70B, lento
```

### Per Milvius veloce
```bash
# Tuning index
nlist: aumenta per dataset grande
nprobe: aumenta per accuratezza
```

---

## 📚 Riferimenti

- [FastAPI Docs](https://fastapi.tiangolo.com)
- [LangChain Docs](https://python.langchain.com)
- [Milvius Docs](https://milvius.io/docs)
- [PaddleOCR Docs](https://github.com/PaddlePaddle/PaddleOCR)
- [Sentence-Transformers](https://www.sbert.net)
- [Ollama](https://ollama.ai)
- [Open WebUI](https://github.com/open-webui/open-webui)

---

## ✅ Conclusione

**Stack scelto è production-ready, scalabile, e rispetta**:
- ✅ Semplicità installazione (Docker one-liner)
- ✅ Protezione IP (offuscamento + containerizzazione)
- ✅ Multi-machine deployable
- ✅ Full autonomia cliente
- ✅ Zero dipendenze cloud (self-hosted)
- ✅ Hardware economico (RTX 5070 = ~400€)

**Pronto per il go-to-market! 🚀**
