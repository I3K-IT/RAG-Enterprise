# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a **RAG (Retrieval-Augmented Generation) Enterprise solution** - a complete production-ready system that combines:
- FastAPI backend with LangChain RAG orchestration
- Open WebUI frontend (ChatGPT-like interface)
- Qdrant vector database for document embeddings
- PaddleOCR for document text extraction
- Ollama for local LLM inference (Mistral/Llama2)
- Docker Compose orchestration

## Architecture

```
┌─────────────────────────────────────────┐
│  Open WebUI Frontend (Port 3000)       │
│  - ChatGPT-like interface               │
│  - Document upload UI                   │
└─────────────────┬───────────────────────┘
                  │ REST API
                  ↓
┌─────────────────────────────────────────┐
│  FastAPI Backend (Port 8000)           │
│  - RAG Pipeline orchestration          │
│  - OCR processing (PaddleOCR)          │
│  - Embedding generation                 │
│  - Document management                  │
└─────────────────┬───────────────────────┘
                  │ gRPC/HTTP
                  ↓
┌─────────────────────────────────────────┐
│  Qdrant Vector Database (Port 6333)    │
│  - Vector storage & similarity search  │
│  - Document chunk retrieval            │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│  Ollama LLM Server (Port 11434)        │
│  - Local LLM inference                 │
│  - Multiple model support              │
└─────────────────────────────────────────┘
```

## Core Components

- **backend/app.py**: Main FastAPI application with endpoints for document upload, querying, and health checks
- **backend/rag_pipeline.py**: LangChain-based RAG orchestration (text chunking, embedding, retrieval, generation)
- **backend/ocr_service.py**: PaddleOCR wrapper for PDF text extraction
- **backend/embeddings_service.py**: Sentence-Transformers embedding generation (supports SOTA models like BGE-M3, DeepSeek)
- **backend/qdrant_connector.py**: Vector database operations and search

## Common Development Commands

### Installation & Setup
```bash
# Full installation (all services)
make install-all
# OR
./setup.sh

# Modular installation
make install-db          # Database only
make install-backend     # Backend + Ollama
make install-frontend    # Frontend only
```

### Service Management
```bash
# View logs
make logs                 # All services
make logs-backend         # Backend only
make logs-frontend        # Frontend only

# Control services
make stop                 # Stop all services
make restart              # Restart all services
make clean                # Remove containers and volumes

# Health check
make health               # Check service status and backend health
```

### Direct Docker Compose Commands
```bash
docker-compose up -d      # Start all services
docker-compose down       # Stop all services
docker-compose ps         # Check service status
docker-compose build      # Rebuild backend image
```

### Development Workflow
```bash
# Backend development
cd rag-enterprise-structure/backend
docker-compose logs -f backend

# Test backend endpoints
curl http://localhost:8000/health
curl http://localhost:8000/docs  # API documentation
```

## Service URLs

- **Frontend**: http://localhost:3000
- **Backend API**: http://localhost:8000
- **API Documentation**: http://localhost:8000/docs
- **Backend Health**: http://localhost:8000/health
- **Qdrant Dashboard**: http://localhost:6333/dashboard

## Key Environment Variables

- `QDRANT_HOST`: Qdrant database host (default: qdrant)
- `LLM_MODEL`: Ollama model name (default: mistral)
- `EMBEDDING_MODEL`: Sentence-Transformers model (default: BAAI/bge-m3)
- `CUDA_VISIBLE_DEVICES`: GPU device ID for acceleration
- `RELEVANCE_THRESHOLD`: Minimum similarity score for retrieval

## Hardware Requirements

- **GPU**: NVIDIA RTX 5070 12GB (critical for performance)
- **RAM**: 32GB minimum (64GB recommended)
- **CPU**: i7/Ryzen 7 quad-core minimum
- **Storage**: 200GB+ SSD

## RAG Pipeline Flow

1. **Document Upload** → OCR text extraction (PaddleOCR)
2. **Text Processing** → Chunking and embedding generation
3. **Vector Storage** → Qdrant indexing
4. **User Query** → Query embedding → Similarity search
5. **Context Retrieval** → Top-k relevant chunks
6. **LLM Generation** → Ollama generates response with sources

## Embedding Models Available

**High Performance Models (Recommended)**:
- **BAAI/bge-m3** (Default): Multilingual SOTA, supports dense+sparse+colbert retrieval, 2.3GB
- **BAAI/bge-large-en-v1.5**: English SOTA performance, 1.3GB
- **intfloat/e5-large-v2**: Multilingual high performance, 1.3GB
- **deepseek-ai/deepseek-coder-6.7b-base**: Specialized for code documents, 13GB

**Balanced Models**:
- **sentence-transformers/all-roberta-large-v1**: English high quality, 1.3GB
- **all-mpnet-base-v2**: English medium quality, 430MB

**Fast Models**:
- **all-MiniLM-L6-v2**: English basic, 22MB, very fast
- **multilingual-MiniLM-L6-v2**: Multilingual basic, 61MB

## Important Notes

- The system uses **Qdrant** as the vector database (not Milvus as mentioned in some docs)
- **BGE-M3** is now the default embedding model for superior performance
- GPU acceleration is essential for OCR and embedding performance
- All services run in isolated Docker networks for security
- The backend includes comprehensive API documentation at `/docs`
- Frontend uses Open WebUI for a ChatGPT-like experience

## File Structure

```
rag-enterprise-structure/
├── docker-compose.yml          # Main orchestration
├── Makefile                   # Development commands
├── setup.sh                   # Installation script
├── backend/
│   ├── app.py                # Main FastAPI app
│   ├── rag_pipeline.py       # RAG orchestration
│   ├── ocr_service.py        # OCR processing
│   ├── embeddings_service.py # Embedding generation
│   ├── qdrant_connector.py   # Vector DB operations
│   ├── requirements.txt      # Python dependencies
│   └── Dockerfile           # Backend container
├── frontend/
│   └── docker-compose.yml    # Standalone frontend
└── database/
    └── docker-compose.yml    # Standalone database
```