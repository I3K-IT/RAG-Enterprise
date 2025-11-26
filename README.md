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
- 🌍 **Multilingual**: Support for 29 languages including Italian
- 🎨 **Modern React UI**: Clean, responsive interface with real-time updates
- 🔐 **User Authentication**: JWT-based auth with role-based access control
- 👥 **Multi-user Support**: User, Super User, and Admin roles
- 📊 **Vector Database**: Qdrant for ultra-fast semantic search
- 🔧 **Document Management**: Upload PDF, DOCX, TXT, MD and other formats
- 💬 **Conversational Memory**: Context-aware responses with chat history

---

## 📋 Architecture

```
┌─────────────────────────────────────────┐
│  React + Vite Frontend (Port 3000)     │
│  - Modern React UI                      │
│  - JWT Authentication                   │
│  - Document upload & management         │
│  - Conversation history                 │
└─────────────────┬───────────────────────┘
                  │ REST API (JWT)
                  ↓
┌─────────────────────────────────────────┐
│  FastAPI Backend (Port 8000)           │
│  - RAG Pipeline (LangChain)            │
│  - User Authentication (JWT)           │
│  - Role-based Access Control           │
│  - OCR (Apache Tika + Tesseract)       │
│  - Embeddings (all-MiniLM-L6-v2)       │
│  - Document Management API             │
└─────────────────┬───────────────────────┘
                  │
    ┌─────────────┴─────────────┐
    ↓                           ↓
┌─────────────────┐   ┌─────────────────┐
│  Qdrant (6333) │   │  Ollama (11434) │
│  Vector DB     │   │  LLM Server     │
│  - 384-dim     │   │  - Mistral 7B   │
│  - Cosine sim  │   │  - Q4 quant     │
└─────────────────┘   └─────────────────┘
```

### User Roles

- **User**: Read-only access, can query documents
- **Super User**: Can upload and delete documents
- **Admin**: Full access including user management

---

## 🚀 Quick Installation

### Prerequisites

- **OS**: Ubuntu 20.04+ (22.04 recommended)
- **GPU**: NVIDIA with 8-16GB VRAM (drivers installed)
- **RAM**: 16GB minimum, 32GB recommended
- **Storage**: 50GB+ SSD
- **Software**: Docker + Docker Compose, NVIDIA Container Toolkit

### Setup with Docker Compose

```bash
# 1. Clone repository
git clone https://github.com/your-org/rag-enterprise.git
cd rag-enterprise/rag-enterprise-structure

# 2. Start all services
docker compose up -d

# 3. Wait for services to be ready (2-3 minutes first run)
# Backend will download models automatically

# 4. Access the application
# Frontend: http://localhost:3000
# Backend API: http://localhost:8000
# Default credentials: admin / admin123
```

### First Login

1. Open http://localhost:3000
2. Login with default credentials:
   - Username: `admin`
   - Password: `admin123`
3. **Important**: Change the admin password immediately
4. Create additional users via the Admin panel if needed

---

## 🔧 Configuration

### Branding Customization

You can customize the application branding (logo and company name). See [LOGO_SETUP.md](LOGO_SETUP.md) for detailed instructions.

### Environment Variables

Edit `docker-compose.yml` to configure:

```yaml
environment:
  # LLM Model
  LLM_MODEL: mistral

  # Embedding model
  EMBEDDING_MODEL: all-MiniLM-L6-v2

  # Similarity threshold (0.0-1.0)
  RELEVANCE_THRESHOLD: "0.3"

  # GPU device
  CUDA_VISIBLE_DEVICES: 0
```

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
docker compose logs -f frontend     # Frontend only
docker compose logs -f ollama       # LLM only

# Service control
docker compose ps                   # Container status
docker compose restart backend      # Restart backend
docker compose down                 # Stop all
docker compose up -d                # Restart all

# Health check
curl http://localhost:8000/health   # Backend health
curl http://localhost:6333/         # Qdrant status
curl http://localhost:3000          # Frontend
```

### User Management (via API)

```bash
# Login and get token
curl -X POST http://localhost:8000/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"admin123"}'

# List all users (Admin only)
curl http://localhost:8000/api/auth/users \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### LLM Model Change

```bash
# 1. Pull new model
docker exec ollama ollama pull qwen2.5:14b-instruct-q4_K_M

# 2. Modify docker-compose.yml
# Change LLM_MODEL environment variable

# 3. Restart backend
docker compose restart backend
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

### With STANDARD Setup (RTX 4070, 12GB)

| Metric | Value |
|---------|--------|
| **Generation speed** | 80-100 token/s |
| **Query latency** | 2-4 seconds |
| **Supported documents** | 1,000+ |
| **Chunks per document** | ~10-20 (average PDF) |
| **Similarity search** | <100ms (Qdrant) |
| **Upload throughput** | 1-2 doc/minute |

### Scaling

- **10 documents**: Instant retrieval (<100ms)
- **100 documents**: Fast (<200ms)
- **1,000 documents**: Good (<500ms)
- **10,000+ documents**: Requires larger models and more VRAM

---

## 🛣️ Roadmap

### ✅ Completed (v1.1)
- [x] RAG pipeline with LangChain
- [x] Q4 quantized models support
- [x] React + Vite modern frontend
- [x] JWT authentication system
- [x] Role-based access control (User/Super User/Admin)
- [x] User management interface
- [x] Document upload and management
- [x] Document deletion via UI
- [x] Conversation persistence
- [x] Multi-user with isolation
- [x] UTF-8 encoding fix
- [x] Temperature 0.0 (deterministic)
- [x] Complete REST API

### 🚧 In Development (v1.2)
- [ ] Conversation history management
- [ ] Document preview before upload
- [ ] Batch document upload
- [ ] Advanced search filters
- [ ] Export conversation history

### 🔮 Future (v2.0)
- [ ] Hybrid search (dense + sparse)
- [ ] Re-ranking with cross-encoder
- [ ] Dynamic chunk optimization
- [ ] Support for tables/charts
- [ ] Streaming API for responses
- [ ] Mobile responsive improvements
- [ ] Dark mode
- [ ] Slack/Teams integration

---

## 📄 License

MIT License - see [LICENSE](LICENSE)

---

## 🤝 Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit changes (`git commit -m 'Add amazing feature'`)
4. Push to branch (`git push origin feature/AmazingFeature`)
5. Open Pull Request

---

## 📧 Support

- **Issues**: [GitHub Issues](https://github.com/your-org/rag-enterprise/issues)
- **Documentation**: See LOGO_SETUP.md for branding customization

---

## 🙏 Credits

- **Mistral AI**: LLM model
- **Sentence Transformers**: Embedding models
- **Qdrant**: Vector database
- **Ollama**: Local LLM runtime
- **LangChain**: RAG orchestration
- **Apache Tika**: Document processing
- **React + Vite**: Frontend framework
- **FastAPI**: Backend framework

---

**Happy RAG! 🚀**
