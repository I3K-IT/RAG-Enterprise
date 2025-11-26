# RAG Enterprise - Production-Ready Local RAG System

**Complete 100% local Retrieval-Augmented Generation (RAG) system** for businesses requiring privacy and total control over their data.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)
[![Docker](https://img.shields.io/badge/docker-required-blue.svg)](https://www.docker.com/)

---

## 🎯 Main Features

- ✅ **100% Local**: No data leaves your infrastructure
- 🚀 **Production-Ready**: Tested with 10,000+ document databases
- 🤖 **SOTA 2025 Models**: Qwen2.5, Mistral, Llama3.1 (Q4 quantized)
- 🌍 **Multilingual**: Native Italian support + 28 other languages
- 🎨 **ChatGPT-like UI**: Familiar interface with Open WebUI
- 📊 **Vector Database**: Qdrant for ultra-fast semantic search
- 🔧 **Document Management**: Upload PDF, DOCX, TXT, MD and other formats
- 🎯 **Intelligent Gap Filtering**: Reduces "noise dilution" in multi-document scenarios

---

## 📋 Architecture

```
┌─────────────────────────────────────────┐
│  Open WebUI Frontend (Port 3000)       │
│  - ChatGPT-like interface               │
│  - Drag&drop document upload            │
└─────────────────┬───────────────────────┘
                  │ REST API
                  ↓
┌─────────────────────────────────────────┐
│  FastAPI Backend (Port 8000)           │
│  - RAG Pipeline (LangChain)            │
│  - OCR (Apache Tika + Tesseract)       │
│  - Embeddings (BAAI/bge-m3)            │
│  - Gap-based filtering                 │
└─────────────────┬───────────────────────┘
                  │
    ┌─────────────┴─────────────┐
    ↓                           ↓
┌─────────────────┐   ┌─────────────────┐
│  Qdrant (6333) │   │  Ollama (11434) │
│  Vector DB     │   │  LLM Server     │
│  - 1024-dim    │   │  - Qwen2.5 14B  │
│  - Cosine sim  │   │  - Q4 quant     │
└─────────────────┘   └─────────────────┘
```

---

## 🚀 Quick Installation

### Prerequisites

- **OS**: Ubuntu 20.04+ (22.04 recommended)
- **GPU**: NVIDIA with 8-16GB VRAM (drivers installed)
- **RAM**: 16GB minimum, 32GB recommended
- **Storage**: 50GB+ SSD
- **Software**: Docker + Docker Compose, NVIDIA Container Toolkit

### Setup with Automated Script

```bash
# 1. Clone repository
git clone https://github.com/your-org/rag-enterprise.git
cd rag-enterprise/rag-enterprise-structure

# 2. Run setup (choose profile based on your GPU)
./setup.sh standard    # For 12-16GB VRAM (RTX 4070, RTX 4060 Ti 16GB)
# or
./setup.sh minimal     # For 8-12GB VRAM (RTX 4060, RTX 3060)
# or
./setup.sh advanced    # For 16-24GB VRAM (RTX 4080, RTX 4090)

# 3. Wait for completion (15-20 minutes first run)
# 4. Access http://localhost:3000
```

---

## 🖥️ Hardware Profiles and Models

### 📊 Comparison Table

| Profile | GPU VRAM | RAM | LLM | Embedding | Threshold | Use Case |
|---------|----------|-----|-----|-----------|-----------|----------|
| **MINIMAL** | 8-12GB | 16GB | mistral:7b-q4 (4GB) | mpnet-base (430MB) | 0.40 | Development, small datasets |
| **STANDARD** ⭐ | 12-16GB | 32GB | **qwen2.5:14b-q4 (8GB)** | **bge-m3 (2.3GB)** | **0.35** | **Production, real-world use** |
| **ADVANCED** | 16-24GB | 64GB+ | qwen2.5:32b-q4 (12GB) | instructor-large (1.3GB) | 0.30 | Maximum quality |

### 🎮 Recommended GPUs by Profile

#### MINIMAL (8-12GB VRAM)
- ✅ **RTX 4060** (8GB) - €300-350
- ✅ **RTX 3060** (12GB) - €250-300
- ✅ **RTX 3060 Ti** (8GB) - €300-350
- ⚠️ **RTX 4060 Ti 8GB** - Tight but works

#### STANDARD (12-16GB VRAM) ⭐ RECOMMENDED
- ✅ **RTX 4070** (12GB) - €550-650
- ✅ **RTX 4060 Ti 16GB** (16GB) - €500-550
- ✅ **RTX 4070 Ti** (12GB) - €750-850
- ✅ **RTX 3090** (24GB) - €800-1000 (used)

#### ADVANCED (16-24GB VRAM)
- ✅ **RTX 4080** (16GB) - €1100-1300
- ✅ **RTX 4090** (24GB) - €1800-2200
- ✅ **RTX 3090 Ti** (24GB) - €1000-1200 (used)
- ✅ **A4000** (16GB) - Workstation

---

## 🤖 Available LLM Models

### SOTA 2025 Models (Q4_K_M Quantized)

| Model | Size | VRAM | Parameters | Quality | Speed | Recommended |
|---------|------------|------|-----------|---------|----------|--------------|
| **mistral:7b-instruct-q4** | 4.1GB | 8GB | 7B | ⭐⭐⭐⭐ | ~120 t/s | MINIMAL |
| **qwen2.5:14b-instruct-q4** ⭐ | 8.0GB | 12GB | 14B | ⭐⭐⭐⭐⭐ | ~110 t/s | **STANDARD** |
| **qwen2.5:32b-instruct-q4** | 12GB | 16GB | 32B | ⭐⭐⭐⭐⭐⭐ | ~80 t/s | ADVANCED |

### Why Qwen2.5?

- ✅ **SOTA 2025**: Trained on 18 trillion tokens
- ✅ **Excellent on RAG**: Optimized for retrieval tasks
- ✅ **Multilingual**: 29 languages (perfect Italian)
- ✅ **Fewer hallucinations**: Large context window + better training
- ✅ **Surpasses GPT-3.5**: On standard RAG benchmarks

### Q4_K_M Quantization

- **Q4_K_M**: "Smart" 4-bit quantization
- **Quality**: 95% vs FP16 models (minimal loss)
- **VRAM**: 60% savings compared to non-quantized
- **Speed**: Faster thanks to reduced size
- **K-quant**: Superior method compared to legacy quantization

---

## 🧠 Embedding Models

| Model | Size | Languages | Vector Dimensions | Quality | Usage |
|---------|------------|--------|-------------------|---------|-----|
| **all-mpnet-base-v2** | 430MB | EN | 768 | ⭐⭐⭐ | MINIMAL |
| **BAAI/bge-m3** ⭐ | 2.3GB | Multilingual | 1024 | ⭐⭐⭐⭐⭐ | **STANDARD** |
| **instructor-large** | 1.3GB | Multilingual | 768 | ⭐⭐⭐⭐ | ADVANCED |

**BGE-M3 is the recommended default**: supports dense, sparse, and ColBERT retrieval simultaneously.

---

## 🎛️ Advanced Configuration

### Environment Variables (docker-compose.yml)

```yaml
environment:
  # LLM Model (change based on profile)
  LLM_MODEL: qwen2.5:14b-instruct-q4_K_M

  # Embedding model
  EMBEDDING_MODEL: BAAI/bge-m3

  # Similarity threshold (0.0-1.0)
  # Higher = more restrictive
  # 0.30 = permissive, 0.40 = balanced, 0.50 = restrictive
  RELEVANCE_THRESHOLD: "0.35"

  # GPU device (0 = first GPU, 1 = second, etc.)
  CUDA_VISIBLE_DEVICES: 0
```

### Gap-Based Filtering

The system implements **intelligent gap filtering** to reduce "noise dilution":

```python
# If the best document has:
# - Score >= 50%
# - Gap > 8% compared to the second
# → Filter documents with score < 45%

Example:
Document A: 62%  ← Relevant
Document B: 44%  ← Noise
Gap: 18% (> 8%)
→ Pass only Document A to LLM
```

This prevents irrelevant documents from "diluting" the context and confusing the LLM.

---

## 📚 Supported Formats

### Document Upload

- ✅ **PDF** (via Apache Tika + OCR)
- ✅ **DOCX/DOC** (Microsoft Word)
- ✅ **PPTX/PPT** (PowerPoint)
- ✅ **XLSX/XLS** (Excel)
- ✅ **TXT, MD** (Plain text, Markdown)
- ✅ **ODT** (OpenDocument)
- ✅ **RTF, HTML, XML**

### Automatic OCR

- **Primary**: Apache Tika (fast, accurate)
- **Fallback**: Tesseract (if Tika fails)
- **Encoding**: UTF-8 forced for Italian accented characters

---

## 🔧 Useful Commands

### System Management

```bash
# View logs
docker compose logs -f              # All services
docker compose logs -f backend      # Backend only
docker compose logs -f ollama       # LLM only

# Service control
docker compose ps                   # Container status
docker compose restart backend      # Restart backend
docker compose down                 # Stop all
docker compose up -d                # Restart all

# Health check
curl http://localhost:8000/health   # Backend health
curl http://localhost:6333/         # Qdrant status
```

### Conversational Memory Management

```bash
# View user memory
curl http://localhost:8000/api/admin/memory

# Delete specific user memory
curl -X DELETE http://localhost:8000/api/admin/memory/default

# Delete ALL memory
curl -X DELETE http://localhost:8000/api/admin/memory
```

### LLM Model Change

```bash
# 1. Pull new model
docker exec rag-ollama ollama pull qwen2.5:32b-instruct-q4_K_M

# 2. Modify docker-compose.yml
sed -i 's/LLM_MODEL: .*/LLM_MODEL: qwen2.5:32b-instruct-q4_K_M/' docker-compose.yml

# 3. Restart
docker compose down && docker compose up -d
```

---

## 🐛 Troubleshooting

### Backend not responding

```bash
# Check error logs
docker logs rag-backend --tail 100

# Verify GPU available
docker exec rag-backend nvidia-smi

# Complete restart
docker compose down
docker compose up -d
```

### LLM model not loaded

```bash
# Check available models in Ollama
docker exec rag-ollama ollama list

# Pull model manually
docker exec rag-ollama ollama pull qwen2.5:14b-instruct-q4_K_M
```

### 0 results in queries

```bash
# Check documents in Qdrant
curl http://localhost:6333/collections/rag_documents

# If count = 0, documents were not indexed
# Check upload logs:
docker logs rag-backend | grep "Upload"

# Lower threshold if too high:
# docker-compose.yml → RELEVANCE_THRESHOLD: "0.30"
```

### Corrupted accented characters

```bash
# Fix already applied in current version
# For old documents: delete and reload

# Delete Qdrant collection
docker compose down
docker volume rm rag-enterprise-structure_qdrant-data
docker compose up -d

# Reload documents via frontend
```

---

## 🔐 Privacy Notes

- ✅ **Zero external calls**: No data leaves your server
- ✅ **No analytics**: No tracking or telemetry
- ✅ **Local models**: LLM and embeddings run on-premise
- ✅ **Local database**: Qdrant does not communicate externally
- ✅ **Local logs**: Everything stays in your filesystem

**Ideal for**: law firms, healthcare, finance, public administration, companies with sensitive data.

---

## 📊 Expected Performance

### With STANDARD Profile (RTX 4070, 12GB)

| Metric | Value |
|---------|--------|
| **Generation speed** | 100-130 token/s |
| **Query latency** | 1-3 seconds |
| **Supported documents** | 10,000+ |
| **Chunks per document** | ~10-20 (average PDF) |
| **Similarity search** | <100ms (Qdrant) |
| **Upload throughput** | 1-2 doc/minute |

### Scaling

- **10 documents**: Instant retrieval (<100ms)
- **100 documents**: Fast (<200ms)
- **1,000 documents**: Good (<500ms)
- **10,000+ documents**: Gap filtering essential

---

## 🛣️ Roadmap

### ✅ Completed (v1.0)
- [x] RAG pipeline with LangChain
- [x] Q4 quantized models
- [x] Gap-based filtering
- [x] UTF-8 encoding fix
- [x] Temperature 0.0 (deterministic)
- [x] Conversational memory
- [x] Complete REST API

### 🚧 In Development (v1.1)
- [ ] Redesigned frontend with document management
- [ ] Indexed document visualization
- [ ] Document deletion via UI
- [ ] Conversation persistence
- [ ] Multi-user with isolation

### 🔮 Future (v2.0)
- [ ] Hybrid search (dense + sparse)
- [ ] Re-ranking with cross-encoder
- [ ] Dynamic chunk optimization
- [ ] Support for tables/charts
- [ ] Streaming API for responses
- [ ] Slack/Teams integration

---

## 📄 Licenza

MIT License - vedi [LICENSE](LICENSE)

---

## 🤝 Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit changes (`git commit -m 'Add amazing feature'`)
4. Push to branch (`git push origin feature/AmazingFeature`)
5. Open Pull Request

---

## 📧 Supporto

- **Issues**: [GitHub Issues](https://github.com/your-org/rag-enterprise/issues)
- **Documentazione**: [Wiki](https://github.com/your-org/rag-enterprise/wiki)
- **Discord**: [Community Server](https://discord.gg/your-server)

---

## 🙏 Credits

- **Qwen2.5**: Alibaba Cloud (modello LLM)
- **BGE-M3**: BAAI (embedding model)
- **Qdrant**: Vector database
- **Ollama**: Local LLM runtime
- **LangChain**: RAG orchestration
- **Apache Tika**: Document processing
- **Open WebUI**: Frontend interface

---

**Happy RAG! 🚀**
