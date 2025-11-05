# 🚀 RAG Enterprise - Retrieval Augmented Generation Platform

Sistema completo di RAG (Retrieval Augmented Generation) locale, privacy-first, ottimizzato per GPU NVIDIA.

## ✨ Caratteristiche

- **Backend**: FastAPI + Python
- **Vector Database**: Qdrant (1024 dim, schema-less)
- **Embedding**: BAAI/bge-m3 (multilingua, SOTA)
- **LLM**: Ollama + neural-chat:7b
- **OCR**: Apache Tika per estrazione documenti PDF/DOCX/TXT
- **GPU**: NVIDIA optimized (RTX 5070 Ti 16GB tested)
- **Privacy**: 100% locale, nessun dato verso server esterni
- **Threshold**: 0.35 (ottimizzato per bge-m3)

## 🏗️ Architettura
```
Frontend Open WebUI (3000)
           ↓
Backend FastAPI (8000)
    ├→ Qdrant Vector DB (6333)
    ├→ Ollama LLM Server (11434)
    ├→ Apache Tika OCR (9998)
    └→ Sentence Transformers
```

## 🚀 Quick Start
```bash
cd ~/ai/rag-enterprise-complete/rag-enterprise-structure/
./setup.sh standard
```

Attenderà ~40 minuti per:
- Installare Docker + NVIDIA Toolkit
- Buildare il backend
- Scaricare i modelli (neural-chat 4GB, bge-m3 2.3GB)

## 📊 Profile Disponibili

- **minimal**: all-mpnet-base-v2 + mistral (12-16GB GPU)
- **standard**: BAAI/bge-m3 + neural-chat (24GB GPU) ← **ATTUALE**
- **advanced**: instructor-large + llama2 (48GB+ GPU)

## 🎯 Configurazione Standard (LIVE)
```
Embedding: BAAI/bge-m3 (1024 dim, multilingua)
LLM: neural-chat:7b
Threshold: 0.35
Chunking: 500 char + 100 overlap
Device: CUDA (auto-detected)
```

## 📝 Funzionalità

✅ Upload PDF/DOCX/TXT via API
✅ Estrazione OCR automatica con Tika
✅ Chunking intelligente con overlap
✅ Embedding multilingua SOTA
✅ Vector search con relevance threshold
✅ RAG query con context retrieval
✅ Similarity scores per fonte
✅ Health check e monitoring

## 🔗 Endpoint Principali

- **Health Check**: `http://localhost:8000/health`
- **API Docs**: `http://localhost:8000/docs`
- **Frontend**: `http://localhost:3000`
- **Qdrant Dashboard**: `http://localhost:6333/dashboard`

### API Principali
```bash
# Upload documento
curl -X POST http://localhost:8000/api/documents/upload \
  -F "file=@documento.pdf"

# Query RAG
curl -X POST http://localhost:8000/api/query \
  -H "Content-Type: application/json" \
  -d '{"query": "cosa contiene il documento", "top_k": 5}'
```

## 📦 Tech Stack

- **Python**: 3.10
- **FastAPI**: Web framework
- **Sentence Transformers**: Embedding (BAAI/bge-m3)
- **Qdrant**: Vector database
- **Ollama**: LLM server (neural-chat:7b)
- **Apache Tika**: Document OCR
- **Docker**: Containerization
- **NVIDIA Container Toolkit**: GPU support
- **Open WebUI**: Frontend

## 🛣️ Roadmap

- [ ] Frontend custom per upload directory intera
- [ ] ReRanker (bge-reranker-base)
- [ ] Hybrid Search (BM25 + vector search)
- [ ] Web UI avanzata con drag-drop
- [ ] API authentication
- [ ] Multi-model support switching
- [ ] Batch processing

## 📋 File Principali
```
rag-enterprise-structure/
├── backend/
│   ├── app.py              # FastAPI main
│   ├── ocr_service.py      # Tika integration
│   ├── embeddings_service.py # Sentence Transformers
│   ├── qdrant_connector.py # Vector DB
│   └── rag_pipeline.py     # RAG logic
├── docker-compose.yml      # Container orchestration
├── setup.sh               # Automatic setup
└── README.md              # This file
```

## 🚨 Requisiti Minimi

- **GPU**: 16GB VRAM minimo (RTX 4060 Ti / RTX 5070 Ti)
- **RAM**: 32GB
- **Storage**: 50GB (modelli + database)
- **OS**: Ubuntu 22.04+ con NVIDIA driver

## 🔧 Troubleshooting

### Backend unhealthy
```bash
sudo docker logs rag-backend | tail -50
```

### Tika non risponde
```bash
curl http://localhost:9998/tika
```

### Qdrant connection error
```bash
curl http://localhost:6333/
```

### Ollama model not found
```bash
sudo docker exec rag-ollama ollama list
```

## 📚 Documentazione Aggiuntiva

Vedi `/docs` per:
- Architecture dettagliata
- Performance tuning
- Custom model configuration
- Multi-machine deployment

## 📄 Licenza

Proprietaria - Uso personale

## 👤 Autore & Team

RAG Enterprise Development Team

---

**Status**: ✅ Production Ready
**Last Updated**: Novembre 2025
**Version**: 1.0.0
