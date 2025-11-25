# RAG Enterprise - Sistema RAG Locale Production-Ready

**Sistema completo di Retrieval-Augmented Generation (RAG) 100% locale** per aziende che necessitano di privacy e controllo totale sui propri dati.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)
[![Docker](https://img.shields.io/badge/docker-required-blue.svg)](https://www.docker.com/)

---

## 🎯 Caratteristiche Principali

- ✅ **100% Locale**: Nessun dato esce dalla tua infrastruttura
- 🚀 **Production-Ready**: Testato con database di 10.000+ documenti
- 🤖 **Modelli SOTA 2025**: Qwen2.5, Mistral, Llama3.1 (quantizzati Q4)
- 🌍 **Multilingue**: Supporto italiano nativo + 28 altre lingue
- 🎨 **UI ChatGPT-like**: Interfaccia familiare con Open WebUI
- 📊 **Vector Database**: Qdrant per ricerca semantica ultra-veloce
- 🔧 **Gestione Documenti**: Upload PDF, DOCX, TXT, MD e altri formati
- 🎯 **Gap Filtering Intelligente**: Riduce "noise dilution" in scenari multi-documento

---

## 📋 Architettura

```
┌─────────────────────────────────────────┐
│  Open WebUI Frontend (Port 3000)       │
│  - Interfaccia ChatGPT-like             │
│  - Upload documenti drag&drop           │
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

## 🚀 Installazione Rapida

### Prerequisiti

- **OS**: Ubuntu 20.04+ (consigliato 22.04)
- **GPU**: NVIDIA con 8-16GB VRAM (driver installati)
- **RAM**: 16GB minimo, 32GB consigliato
- **Storage**: 50GB+ SSD
- **Software**: Docker + Docker Compose, NVIDIA Container Toolkit

### Setup con Script Automatico

```bash
# 1. Clone repository
git clone https://github.com/your-org/rag-enterprise.git
cd rag-enterprise/rag-enterprise-structure

# 2. Esegui setup (scegli profilo in base alla tua GPU)
./setup.sh standard    # Per 12-16GB VRAM (RTX 4070, RTX 4060 Ti 16GB)
# oppure
./setup.sh minimal     # Per 8-12GB VRAM (RTX 4060, RTX 3060)
# oppure
./setup.sh advanced    # Per 16-24GB VRAM (RTX 4080, RTX 4090)

# 3. Attendi completamento (15-20 minuti primo avvio)
# 4. Accedi a http://localhost:3000
```

---

## 🖥️ Profili Hardware e Modelli

### 📊 Tabella Comparativa

| Profilo | GPU VRAM | RAM | LLM | Embedding | Threshold | Use Case |
|---------|----------|-----|-----|-----------|-----------|----------|
| **MINIMAL** | 8-12GB | 16GB | mistral:7b-q4 (4GB) | mpnet-base (430MB) | 0.40 | Sviluppo, piccoli dataset |
| **STANDARD** ⭐ | 12-16GB | 32GB | **qwen2.5:14b-q4 (8GB)** | **bge-m3 (2.3GB)** | **0.35** | **Production, uso reale** |
| **ADVANCED** | 16-24GB | 64GB+ | qwen2.5:32b-q4 (12GB) | instructor-large (1.3GB) | 0.30 | Massima qualità |

### 🎮 GPU Consigliate per Profilo

#### MINIMAL (8-12GB VRAM)
- ✅ **RTX 4060** (8GB) - €300-350
- ✅ **RTX 3060** (12GB) - €250-300
- ✅ **RTX 3060 Ti** (8GB) - €300-350
- ⚠️ **RTX 4060 Ti 8GB** - Stretto ma funziona

#### STANDARD (12-16GB VRAM) ⭐ RACCOMANDATO
- ✅ **RTX 4070** (12GB) - €550-650
- ✅ **RTX 4060 Ti 16GB** (16GB) - €500-550
- ✅ **RTX 4070 Ti** (12GB) - €750-850
- ✅ **RTX 3090** (24GB) - €800-1000 (usata)

#### ADVANCED (16-24GB VRAM)
- ✅ **RTX 4080** (16GB) - €1100-1300
- ✅ **RTX 4090** (24GB) - €1800-2200
- ✅ **RTX 3090 Ti** (24GB) - €1000-1200 (usata)
- ✅ **A4000** (16GB) - Workstation

---

## 🤖 Modelli LLM Disponibili

### Modelli SOTA 2025 (Quantizzati Q4_K_M)

| Modello | Dimensioni | VRAM | Parametri | Qualità | Velocità | Raccomandato |
|---------|------------|------|-----------|---------|----------|--------------|
| **mistral:7b-instruct-q4** | 4.1GB | 8GB | 7B | ⭐⭐⭐⭐ | ~120 t/s | MINIMAL |
| **qwen2.5:14b-instruct-q4** ⭐ | 8.0GB | 12GB | 14B | ⭐⭐⭐⭐⭐ | ~110 t/s | **STANDARD** |
| **qwen2.5:32b-instruct-q4** | 12GB | 16GB | 32B | ⭐⭐⭐⭐⭐⭐ | ~80 t/s | ADVANCED |

### Perché Qwen2.5?

- ✅ **SOTA 2025**: Training su 18 trilioni di token
- ✅ **Eccellente su RAG**: Ottimizzato per retrieval tasks
- ✅ **Multilingue**: 29 lingue (italiano perfetto)
- ✅ **Meno allucinazioni**: Context window grande + training migliore
- ✅ **Supera GPT-3.5**: Su benchmark RAG standard

### Quantizzazione Q4_K_M

- **Q4_K_M**: Quantizzazione "smart" a 4-bit
- **Qualità**: 95% vs modelli FP16 (perdita minima)
- **VRAM**: 60% risparmio rispetto a non quantizzati
- **Velocità**: Più veloce grazie a dimensioni ridotte
- **K-quant**: Metodo superiore rispetto a legacy quantization

---

## 🧠 Modelli Embedding

| Modello | Dimensioni | Lingue | Dimensioni Vector | Qualità | Uso |
|---------|------------|--------|-------------------|---------|-----|
| **all-mpnet-base-v2** | 430MB | EN | 768 | ⭐⭐⭐ | MINIMAL |
| **BAAI/bge-m3** ⭐ | 2.3GB | Multilingual | 1024 | ⭐⭐⭐⭐⭐ | **STANDARD** |
| **instructor-large** | 1.3GB | Multilingual | 768 | ⭐⭐⭐⭐ | ADVANCED |

**BGE-M3 è il default consigliato**: supporta retrieval denso, sparso e ColBERT simultaneamente.

---

## 🎛️ Configurazione Avanzata

### Variabili Ambiente (docker-compose.yml)

```yaml
environment:
  # Modello LLM (cambia in base al profilo)
  LLM_MODEL: qwen2.5:14b-instruct-q4_K_M

  # Modello embedding
  EMBEDDING_MODEL: BAAI/bge-m3

  # Threshold similarità (0.0-1.0)
  # Più alto = più restrittivo
  # 0.30 = permissivo, 0.40 = bilanciato, 0.50 = restrittivo
  RELEVANCE_THRESHOLD: "0.35"

  # GPU device (0 = prima GPU, 1 = seconda, etc.)
  CUDA_VISIBLE_DEVICES: 0
```

### Gap-Based Filtering

Il sistema implementa un **gap filtering intelligente** per ridurre "noise dilution":

```python
# Se il documento migliore ha:
# - Score >= 50%
# - Gap > 8% rispetto al secondo
# → Filtra documenti con score < 45%

Esempio:
Documento A: 62%  ← Rilevante
Documento B: 44%  ← Rumore
Gap: 18% (> 8%)
→ Passa solo Documento A all'LLM
```

Questo previene che documenti irrilevanti "diluiscano" il contesto e confondano l'LLM.

---

## 📚 Formati Supportati

### Upload Documenti

- ✅ **PDF** (via Apache Tika + OCR)
- ✅ **DOCX/DOC** (Microsoft Word)
- ✅ **PPTX/PPT** (PowerPoint)
- ✅ **XLSX/XLS** (Excel)
- ✅ **TXT, MD** (Plain text, Markdown)
- ✅ **ODT** (OpenDocument)
- ✅ **RTF, HTML, XML**

### OCR Automatico

- **Primary**: Apache Tika (veloce, preciso)
- **Fallback**: Tesseract (se Tika fallisce)
- **Encoding**: UTF-8 forzato per caratteri accentati italiani

---

## 🔧 Comandi Utili

### Gestione Sistema

```bash
# Visualizza log
docker compose logs -f              # Tutti i servizi
docker compose logs -f backend      # Solo backend
docker compose logs -f ollama       # Solo LLM

# Controllo servizi
docker compose ps                   # Status containers
docker compose restart backend      # Riavvia backend
docker compose down                 # Ferma tutto
docker compose up -d                # Riavvia tutto

# Verifica salute
curl http://localhost:8000/health   # Backend health
curl http://localhost:6333/         # Qdrant status
```

### Gestione Memoria Conversazionale

```bash
# Visualizza memoria utenti
curl http://localhost:8000/api/admin/memory

# Cancella memoria utente specifico
curl -X DELETE http://localhost:8000/api/admin/memory/default

# Cancella TUTTA la memoria
curl -X DELETE http://localhost:8000/api/admin/memory
```

### Cambio Modello LLM

```bash
# 1. Pull nuovo modello
docker exec rag-ollama ollama pull qwen2.5:32b-instruct-q4_K_M

# 2. Modifica docker-compose.yml
sed -i 's/LLM_MODEL: .*/LLM_MODEL: qwen2.5:32b-instruct-q4_K_M/' docker-compose.yml

# 3. Restart
docker compose down && docker compose up -d
```

---

## 🐛 Troubleshooting

### Backend non risponde

```bash
# Controlla log errori
docker logs rag-backend --tail 100

# Verifica GPU disponibile
docker exec rag-backend nvidia-smi

# Riavvio completo
docker compose down
docker compose up -d
```

### Modello LLM non caricato

```bash
# Verifica modelli disponibili in Ollama
docker exec rag-ollama ollama list

# Pull modello manualmente
docker exec rag-ollama ollama pull qwen2.5:14b-instruct-q4_K_M
```

### 0 risultati nelle query

```bash
# Verifica documenti in Qdrant
curl http://localhost:6333/collections/rag_documents

# Se count = 0, i documenti non sono stati indicizzati
# Controlla log upload:
docker logs rag-backend | grep "Upload"

# Abbassa threshold se troppo alto:
# docker-compose.yml → RELEVANCE_THRESHOLD: "0.30"
```

### Caratteri accentati corrotti

```bash
# Fix già applicato in versione corrente
# Per documenti vecchi: elimina e ricarica

# Elimina collection Qdrant
docker compose down
docker volume rm rag-enterprise-structure_qdrant-data
docker compose up -d

# Ricarica documenti via frontend
```

---

## 🔐 Note sulla Privacy

- ✅ **Zero chiamate esterne**: Nessun dato esce dal tuo server
- ✅ **Nessun analytics**: Nessun tracking o telemetria
- ✅ **Modelli locali**: LLM e embeddings girano on-premise
- ✅ **Database locale**: Qdrant non comunica con l'esterno
- ✅ **Logs locali**: Tutto rimane nel tuo filesystem

**Ideale per**: studi legali, sanità, finanza, pubblica amministrazione, aziende con dati sensibili.

---

## 📊 Performance Attese

### Con Profilo STANDARD (RTX 4070, 12GB)

| Metrica | Valore |
|---------|--------|
| **Velocità generazione** | 100-130 token/s |
| **Latenza query** | 1-3 secondi |
| **Documenti supportati** | 10.000+ |
| **Chunks per documento** | ~10-20 (PDF medio) |
| **Similarity search** | <100ms (Qdrant) |
| **Throughput upload** | 1-2 doc/minuto |

### Scaling

- **10 documenti**: Instant retrieval (<100ms)
- **100 documenti**: Veloce (<200ms)
- **1.000 documenti**: Buono (<500ms)
- **10.000+ documenti**: Gap filtering essenziale

---

## 🛣️ Roadmap

### ✅ Completato (v1.0)
- [x] RAG pipeline con LangChain
- [x] Modelli quantizzati Q4
- [x] Gap-based filtering
- [x] UTF-8 encoding fix
- [x] Temperature 0.0 (deterministico)
- [x] Memoria conversazionale
- [x] API REST completa

### 🚧 In Sviluppo (v1.1)
- [ ] Frontend riprogettato con gestione documenti
- [ ] Visualizzazione documenti indicizzati
- [ ] Cancellazione documenti via UI
- [ ] Persistenza conversazioni
- [ ] Multi-utente con isolamento

### 🔮 Futuro (v2.0)
- [ ] Hybrid search (dense + sparse)
- [ ] Re-ranking con cross-encoder
- [ ] Chunk optimization dinamico
- [ ] Support per tabelle/grafici
- [ ] API streaming per risposte
- [ ] Integrazione Slack/Teams

---

## 📄 Licenza

MIT License - vedi [LICENSE](LICENSE)

---

## 🤝 Contributing

Contributions are welcome! Per favore:

1. Fork il repository
2. Crea feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit changes (`git commit -m 'Add amazing feature'`)
4. Push al branch (`git push origin feature/AmazingFeature`)
5. Apri Pull Request

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

**Buon RAG! 🚀**
