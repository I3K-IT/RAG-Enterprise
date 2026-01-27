# RAG Enterprise - Local RAG System

**100% local Retrieval-Augmented Generation (RAG) system** for businesses that need complete data privacy and control.

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)
[![Docker](https://img.shields.io/badge/docker-required-blue.svg)](https://www.docker.com/)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

> **One command. Full RAG system. Your data stays yours.**

---

## Why RAG Enterprise?

- ✅ **100% Local**: No data leaves your infrastructure
- 🚀 **One-Command Setup**: Automated installation script (~1 hour / ~15 min with fast connection)
- 🤖 **Modern LLMs**: Qwen2.5, Mistral 7B (Q4 quantized)
- 🔐 **Multi-user Auth**: JWT-based with role-based access control
- 📊 **Production-Ready**: Designed for 10,000+ documents
- 🌍 **Multilingual**: Supports 29 languages
- 🎨 **Clean UI**: Modern React interface
- 📁 **Multiple Formats**: PDF, DOCX, TXT, MD, PPTX, XLSX, and more

---

## Quick Start

### Prerequisites

- **OS**: Ubuntu 20.04+ (22.04 recommended)
- **GPU**: NVIDIA with 8-16GB VRAM (drivers pre-installed)
- **RAM**: 16GB minimum, 32GB recommended
- **Storage**: 50GB+ available
- **Connection**: 80+ Mbit/s recommended

### Automated Installation

```bash
# 1. Clone repository
git clone https://github.com/your-org/rag-enterprise.git
cd rag-enterprise/rag-enterprise-structure

# 2. Run setup script (installs everything)
./setup.sh standard

# 3. Follow prompts - you'll need to logout/login once during setup
# Setup time: ~1 hour (80 Mbit/s) / ~10-15 min (400+ Mbit/s)

# 4. Access the application
# Frontend: http://localhost:3000
# Get admin password: docker compose logs backend | grep "Password:"
```

### What Gets Installed

The setup script automatically installs and configures:
- Docker Engine + Docker Compose
- NVIDIA Container Toolkit
- All required Docker images
- Ollama with LLM model (Mistral 7B or Qwen2.5:14b)
- Qdrant vector database
- Backend + Frontend services

**First Startup**: After setup completes, the backend downloads the embedding model (~2.3GB) on first startup. This takes ~9 minutes (80 Mbit/s) or ~2 minutes (400+ Mbit/s). Check status with:

```bash
docker compose logs backend -f
```

Once you see "Application startup complete", open http://localhost:3000 and login.

---

## Architecture

```
┌─────────────────────────────────────────┐
│  React + Vite Frontend (Port 3000)     │
│  - JWT Authentication                   │
│  - Document Management                  │
│  - Conversation History                 │
└─────────────────┬───────────────────────┘
                  │ REST API
                  ↓
┌─────────────────────────────────────────┐
│  FastAPI Backend (Port 8000)           │
│  - RAG Pipeline (LangChain)            │
│  - Role-Based Access Control           │
│  - OCR (Apache Tika + Tesseract)       │
│  - Embeddings (BAAI/bge-m3)            │
└─────────────────┬───────────────────────┘
                  │
    ┌─────────────┴─────────────┐
    ↓                           ↓
┌─────────────────┐   ┌─────────────────┐
│  Qdrant (6333) │   │  Ollama (11434) │
│  Vector DB     │   │  LLM Server     │
└─────────────────┘   └─────────────────┘
```

### User Roles

- **User**: Query documents (read-only)
- **Super User**: Upload and delete documents
- **Admin**: Full access including user management

---

## Usage

### First Login

1. Open http://localhost:3000
2. Get your admin password from the logs:
   ```bash
   docker compose logs backend | grep "Password:"
   ```
3. Login with username `admin` and the password from logs
4. Create additional users in Admin panel

### Upload Documents

1. Login as Super User or Admin
2. Click "Upload Document"
3. Select files (PDF, DOCX, TXT, MD, etc.)
4. Wait for processing (1-2 min per document)
5. Start querying your documents

### Supported Formats

- ✅ PDF (with OCR)
- ✅ DOCX/DOC
- ✅ PPTX/PPT
- ✅ XLSX/XLS
- ✅ TXT, MD
- ✅ ODT, RTF, HTML, XML

---

## Configuration

### Change LLM Model

Edit `docker-compose.yml`:

```yaml
environment:
  LLM_MODEL: qwen2.5:14b-instruct-q4_K_M  # or mistral:7b-instruct-q4_K_M
  EMBEDDING_MODEL: BAAI/bge-m3
  RELEVANCE_THRESHOLD: "0.35"
```

Then restart:
```bash
docker compose restart backend
```

### Customize Branding

See [LOGO_SETUP.md](LOGO_SETUP.md) for logo and company name customization.

---

## Useful Commands

### System Management

```bash
# View all logs
docker compose logs -f

# View backend logs only
docker compose logs -f backend

# Check service status
docker compose ps

# Restart services
docker compose restart

# Stop everything
docker compose down

# Start everything
docker compose up -d

# Health checks
curl http://localhost:8000/health
curl http://localhost:3000
```

### Cleanup & Reinstall

If you need to start fresh:

```bash
# Complete cleanup (removes everything)
./cleanup.sh

# Logout and login again

# Run setup from scratch
./setup.sh standard
```

---

## Troubleshooting

### Backend shows "unhealthy"

Wait 3-5 minutes on first startup - it's downloading the embedding model:

```bash
docker compose logs backend -f
```

Look for "Application startup complete" message.

### Can't login / Frontend not loading

Check all services are running:

```bash
docker compose ps

# All should show "Up" status
# If backend is "unhealthy", wait a few more minutes
```

### GPU not detected

Verify NVIDIA drivers and Container Toolkit:

```bash
nvidia-smi
docker run --rm --gpus all nvidia/cuda:12.9.0-runtime-ubuntu22.04 nvidia-smi
```

### No results from queries

Lower the similarity threshold in `docker-compose.yml`:

```yaml
RELEVANCE_THRESHOLD: "0.3"  # Lower = more results
```

Then `docker compose restart backend`.

---

## Performance

### Expected Speed (RTX 4070, 12GB VRAM)

- **Setup time**: ~1 hour (80 Mbit/s) / ~10-15 min (400+ Mbit/s)
- **First startup**: +9 min (80 Mbit/s) / +2 min (400+ Mbit/s) for embedding model
- **Total ready**: ~1h 10min (80 Mbit/s) / ~15-20 min (fast connection)
- **Query response**: 2-4 seconds
- **Generation speed**: 80-100 tokens/s
- **Document capacity**: 1,000-10,000 documents
- **Upload speed**: 1-2 documents/minute

---

## Community Benchmarks

Real-world performance results from the community. Run `python benchmark/rag_benchmark.py` to test your hardware and contribute your results!

### Hardware Configurations Tested

| Hardware | CPU | RAM | GPU | VRAM |
|----------|-----|-----|-----|------|
| **Config #1** | AMD Ryzen 9 5950X (32 cores) | 64 GB | RTX 5070 Ti | 16 GB |

### Upload Performance (seconds)

| Hardware | Mean | Median | Min | Max |
|----------|------|--------|-----|-----|
| **Config #1** | 10.96s | 9.31s | 0.59s | 24.62s |

### Query Performance (seconds)

| Hardware | Mean | Median | Min | Max | P95 |
|----------|------|--------|-----|-----|-----|
| **Config #1** | 4.31s | 3.63s | 2.09s | 8.78s | 8.78s |

### Test Documents

| Document | Type | Size | Upload Time (Config #1) |
|----------|------|------|-------------------------|
| Mueller Report (2019) | Legal | 11 MB | 17.72s |
| 9/11 Commission Report | Legal | 7.4 MB | 24.62s |
| Bitcoin Whitepaper | Technical | 180 KB | 0.59s |
| Attention Is All You Need | Technical | 2.2 MB | 0.91s |

### Query Accuracy (Similarity Scores)

| Query Type | Config #1 |
|------------|-----------|
| Factual lookups | 65-69% |
| Conceptual questions | 50-62% |
| Complex analysis | 45-55% |

> **Want to contribute?** Run the benchmark on your hardware and [open an issue](https://github.com/I3K-IT/RAG-Enterprise/issues) with your results!
> See [benchmark/README.md](benchmark/README.md) for instructions.

---

## Privacy & Security

- ✅ **Zero external calls**: Everything runs locally
- ✅ **No telemetry**: No tracking or analytics
- ✅ **Local models**: LLM and embeddings on-premise
- ✅ **AGPL-3.0 License**: If you modify and deploy as a service, you must share source code

**Ideal for**: Law firms, healthcare, finance, government, enterprises with sensitive data.

---

## License

This project is licensed under **AGPL-3.0** - see [LICENSE](LICENSE) file.

**What this means:**
- ✅ Free to use and modify
- ✅ Must share modifications if you offer it as a service
- ✅ Protects against proprietary SaaS parasites
- ✅ Still fully open-source

---

## Contributing

We welcome contributions! Please read our guidelines:

- [Contributing Guide](CONTRIBUTING.md) - How to contribute
- [Code of Conduct](CODE_OF_CONDUCT.md) - Community guidelines
- [Security Policy](SECURITY.md) - Report vulnerabilities
- [Roadmap](ROADMAP.md) - Planned features

Quick start:
1. Fork the repository
2. Create feature branch (`git checkout -b feature/YourFeature`)
3. Make your changes
4. Open Pull Request

---

## Support

- **Issues**: [GitHub Issues](https://github.com/I3K-IT/RAG-Enterprise/issues)
- **Discussions**: [GitHub Discussions](https://github.com/I3K-IT/RAG-Enterprise/discussions)
- **Docs**: See [LOGO_SETUP.md](LOGO_SETUP.md) for branding customization

---

## Star History

If you find this project useful, please consider giving it a star! It helps others discover the project.

[![Star History Chart](https://api.star-history.com/svg?repos=I3K-IT/RAG-Enterprise&type=Date)](https://star-history.com/#I3K-IT/RAG-Enterprise&Date)

---

## Credits

Built with:
- [Ollama](https://ollama.ai) - Local LLM runtime
- [Qdrant](https://qdrant.tech) - Vector database
- [LangChain](https://langchain.com) - RAG orchestration
- [FastAPI](https://fastapi.tiangolo.com) - Backend framework
- [React](https://react.dev) + [Vite](https://vitejs.dev) - Frontend
- [Apache Tika](https://tika.apache.org) - Document processing

---

## Author

**Francesco Marchetti** - Creator & Lead Developer

- LinkedIn: [francesco-marchetti](https://www.linkedin.com/in/francesco-marchetti-4a7b8149/)
- Company: [I3K Technologies](https://www.i3k.eu)
- Email: info@i3k.eu
- Phone: +39 02 99914444 | +39 338 7913579

---

**Made with ❤️ by [I3K Technologies](https://www.i3k.eu) for privacy-conscious enterprises**
