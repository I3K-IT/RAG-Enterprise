# RAG Enterprise - Architettura Multi-Machine

## Panoramica

Sistema RAG distribuito con componenti separabili:

```
┌─────────────────────────────────────────────────────────────┐
│                     MILVIUS DB (CENTRO)                     │
│              Shared Vector Database (12-001 TCP)             │
│    Accessibile da backend, frontend e altre locations       │
└─────────────────────────────────────────────────────────────┘
          ↗️                            ↖️
    [Condiviso]                   [Condiviso]
          ↗️                            ↖️

┌──────────────────────────┐      ┌──────────────────────────┐
│   BACKEND (Macchina 1)   │      │  FRONTEND (Macchina 2)   │
│  - OCR Processing        │      │  - Open WebUI            │
│  - Embedding Generation  │      │  - API Client            │
│  - Document Indexing     │      │  - User Interface        │
│  - RAG Pipeline (FastAPI)│      │  (Legge da Milvius)      │
│  (Scrive su Milvius)     │      │  (Query via Backend)      │
│                          │      │                          │
│  RTX 5070 (12GB VRAM)    │      │  CPU-only ok             │
│  32-64GB RAM             │      │  8-16GB RAM sufficiente   │
└──────────────────────────┘      └──────────────────────────┘
          ↓                                  ↓
    API :8000                          :3000 UI
```

## Configurazione

### Scenario 1: Tutto Locale
- Milvius: `localhost:19530`
- Backend: `localhost:8000`
- Frontend: `localhost:3000`

### Scenario 2: Backend e Frontend Separati (Same Network)
- Milvius: `milvius-server:19530` (su macchina dedicata o container)
- Backend: `backend-server:8000`
- Frontend: accede a `http://backend-server:8000` per API

### Scenario 3: Multi-Location (Remote)
- Milvius: `milvius.datacenter.com:19530`
- Backend: `backend.datacenter.com:8000`
- Frontend: `frontend.local:3000` → chiama `http://backend.datacenter.com:8000`

## Componenti

### Backend (`/backend`)
- FastAPI server
- LangChain RAG pipeline
- OCR service (PaddleOCR)
- Embedding generation (Sentence-Transformers)
- Connessione a Milvius

### Frontend (`/frontend`)
- Open WebUI instance
- Configurazione per remote backend
- Nessuna dipendenza da GPU

### Database (`/database`)
- Docker Compose per Milvius standalone
- Volumes persistenti
- Configurazione per remote access

### Setup (`/setup`)
- Script di installazione per ogni componente
- Configuration templates
- Utility per testing

## Installazione Rapida

### Opzione 1: Monolith (Tutto insieme)
```bash
./setup/install-all.sh
```

### Opzione 2: Distribuit
```bash
# Macchina 1 - Milvius + Backend
./setup/install-backend-only.sh --with-db

# Macchina 2 - Frontend
./setup/install-frontend-only.sh --backend-url http://backend-server:8000
```

### Opzione 3: Database Separato
```bash
# Macchina 1 - Solo Milvius
./setup/install-db-only.sh

# Macchina 2 - Backend
./setup/install-backend-only.sh --db-host milvius-server

# Macchina 3 - Frontend
./setup/install-frontend-only.sh --backend-url http://backend-server:8000
```

## Variabili di Configurazione

Vedi `.env.example` per tutte le opzioni disponibili.

### Critiche:
- `MILVIUS_HOST`: Indirizzo del server Milvius
- `MILVIUS_PORT`: Porta Milvius (default 19530)
- `BACKEND_URL`: URL del backend per frontend
- `LLM_MODEL`: Modello LLM da usare
- `EMBEDDING_MODEL`: Modello embedding
- `CUDA_VISIBLE_DEVICES`: GPU da usare

## Architettura Dettagliata

Vedi `/docs/architecture.md` per approfondimenti su:
- Flusso dati
- Protocolli di comunicazione
- Sicurezza e autenticazione
- Performance tuning

## Support

Per problemi di installazione, vedi `/docs/troubleshooting.md`
