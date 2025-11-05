# RAG Enterprise - Setup Profiles

Sistema di installazione modulare con profili predefiniti per diverse esigenze hardware e casi d'uso.

---

## 📊 Profili Disponibili

### 1️⃣ **MINIMAL** (Piccole aziende, basso carico)
**Hardware:** CPU 4-core, RAM 8GB, GPU opzionale  
**Caso d'uso:** Database aziendale piccolo, query occasional, analisi interna

| Componente | Modello | Size | Scopo |
|-----------|---------|------|-------|
| **Embedding** | all-MiniLM-L6-v2 | 22MB | Ricerca semantica veloce |
| **LLM** | phi-2 | 1.4GB | Generazione testo leggera |
| **OCR** | PaddleOCR EN | 100MB | Estrazione testo da doc |
| **DB** | Qdrant | 500MB | Vector storage |
| **Tempo setup** | ~10 min | | |
| **Spazio totale** | ~2.5GB | | |

**Perfetto per:**
- Startup con budget limitato
- Analisi interna documenti
- Prototipazione
- Ambienti test

---

### 2️⃣ **STANDARD** (Medie aziende, carico medio) ⭐ [DEFAULT ATTUALE]
**Hardware:** CPU 8-core, RAM 16GB, GPU con 6GB VRAM  
**Caso d'uso:** DB aziendale medio, query frequente, RAG production-ready

| Componente | Modello | Size | Scopo |
|-----------|---------|------|-------|
| **Embedding** | all-mpnet-base-v2 | 438MB | Ricerca semantica accurata |
| **LLM** | mistral | 4GB | Generazione testo qualitativa |
| **OCR** | PaddleOCR EN v4 | 120MB | Estrazione testo accurata |
| **DB** | Qdrant | 1GB | Vector storage scalabile |
| **Tempo setup** | ~25 min | | |
| **Spazio totale** | ~5.5GB | | |

**Perfetto per:**
- Aziende in crescita
- Produzione RAG
- Query complesse
- Documentazione aziendale

---

### 3️⃣ **ADVANCED** (Grandi aziende, alto carico)
**Hardware:** CPU 16-core, RAM 32GB, GPU con 24GB VRAM (RTX 4090/L40)  
**Caso d'uso:** DB massivo, query real-time, alta precisione, multi-user

| Componente | Modello | Size | Scopo |
|-----------|---------|------|-------|
| **Embedding** | bge-large-en-v1.5 | 1.3GB | Ricerca semantica SOTA |
| **LLM** | neural-chat-7b | 4GB | Conversazione avanzata |
| **LLM-Ragù** | mistral-7b-instruct | 4GB | Specializzato per RAG |
| **OCR** | PaddleOCR + layout | 200MB | Estrazione con struttura |
| **DB** | Qdrant cluster | 5GB+ | Vector store distribuito |
| **Tempo setup** | ~40 min | | |
| **Spazio totale** | ~15GB | | |

**Perfetto per:**
- Grandi aziende
- Multi-tenant
- SLA stringenti
- Real-time processing

---

### 4️⃣ **EXPERT** (Massima qualità, ricerca/università)
**Hardware:** GPU 48GB+ (A100/H100), CPU 32-core, RAM 128GB  
**Caso d'uso:** Ricerca, precisione massima, modelli custom

| Componente | Modello | Size | Scopo |
|-----------|---------|------|-------|
| **Embedding** | instructor-large | 1.5GB | Ricerca con istruzioni |
| **LLM** | llama-2-70b-chat | 39GB | Generazione SOTA |
| **LLM-Small** | mistral-7b | 4GB | Fallback veloce |
| **OCR** | EasyOCR + PaddleOCR | 500MB | Multi-language |
| **DB** | Qdrant cluster + backup | 10GB+ | Enterprise-grade |
| **Tempo setup** | ~60 min | | |
| **Spazio totale** | ~55GB | | |

**Perfetto per:**
- Research institutions
- Enterprise mission-critical
- Massima accuratezza
- Custom fine-tuning

---

## 💾 Matrice Hardware vs Profili

```
                  MINIMAL    STANDARD   ADVANCED   EXPERT
CPU cores         4-8        8-16       16-32      32+
RAM              8-16GB     16-32GB    32-64GB    128GB+
GPU VRAM         -/4GB      6-8GB      24GB       48GB+
Storage          10GB       20GB       50GB       100GB+
Network          1Gbps      10Gbps     10Gbps+    40Gbps+
Concurrent users 1-2        5-10       50-100     500+
Query/sec        1-2        10-20      100+       1000+
```

---

## 🎯 Combinazioni Embedding + LLM Consigliate

### ✅ **MINIMAL Setup**
```
Embedding: all-MiniLM-L6-v2 (22MB)
LLM: phi-2 (1.4GB)
Total: 1.4GB LLM + 22MB embedding = 1.5GB attivo
```
**Quando usarlo:** Demo, test locale, risorse limitate

---

### ✅ **STANDARD Setup** (CONSIGLIATO)
```
Embedding: all-mpnet-base-v2 (438MB)
LLM: mistral-7b-instruct (4GB)
Total: 4GB LLM + 438MB embedding = 4.4GB attivo
```
**Quando usarlo:** Produzione piccola/media, best value/performance

---

### ✅ **ADVANCED Setup**
```
Embedding: bge-large-en-v1.5 (1.3GB)
LLM Primary: neural-chat-7b-v3-2 (4GB)
LLM Secondary: mistral-7b (4GB) - fallback
Total: 8GB LLM + 1.3GB embedding = 9.3GB attivo
```
**Quando usarlo:** Produzione alta qualità, multi-user

---

### ✅ **EXPERT Setup**
```
Embedding: instructor-large (1.5GB)
LLM Primary: llama-2-70b-chat-hf (39GB)
LLM Fallback: mistral-7b (4GB)
Total: 43GB + 1.5GB = 44.5GB attivo
```
**Quando usarlo:** Massima qualità, ricerca, enterprise mission-critical

---

## 📋 Dettagli Modelli per Profilo

### MINIMAL - phi-2
- **Parametri:** 2.7B
- **Velocità:** Molto veloce (~5s per query)
- **Qualità:** Base, buona per summarization
- **VRAM:** 4-6GB
- **Use case:** Internal docs, Q&A semplice

### STANDARD - mistral-7b-instruct
- **Parametri:** 7B
- **Velocità:** Veloce (~3s per query)
- **Qualità:** Buona, specializzato per instructions
- **VRAM:** 6-8GB
- **Use case:** Production RAG, conversazioni

### ADVANCED - neural-chat-7b-v3
- **Parametri:** 7B
- **Velocità:** Veloce (~3s per query)
- **Qualità:** Molto buona, conversazioni naturali
- **VRAM:** 6-8GB
- **Use case:** Chat, assistente conversazionale

### EXPERT - llama-2-70b-chat
- **Parametri:** 70B
- **Velocità:** Lenta (~10-15s per query)
- **Qualità:** SOTA, instruction following perfetto
- **VRAM:** 39GB (int8) / 78GB (fp16)
- **Use case:** Massima accuratezza, ricerca

---

## 🔧 Come Scegliere il Profilo

### Domande da Porsi:

1. **Quanti documenti?**
   - < 1000 docs → MINIMAL
   - 1000-10000 docs → STANDARD
   - 10000-100000 docs → ADVANCED
   - 100000+ docs → EXPERT

2. **Quanti utenti simultanei?**
   - 1-2 → MINIMAL
   - 3-10 → STANDARD
   - 11-50 → ADVANCED
   - 50+ → EXPERT

3. **Latenza richiesta?**
   - < 5s → MINIMAL/STANDARD
   - < 2s → STANDARD/ADVANCED
   - < 500ms → ADVANCED/EXPERT

4. **Budget disponibile?**
   - Limitato → MINIMAL
   - Medio → STANDARD
   - Alto → ADVANCED
   - Illimitato → EXPERT

5. **Accuratezza richiesta?**
   - Base (70%) → MINIMAL
   - Buona (85%) → STANDARD
   - Molto buona (92%) → ADVANCED
   - Massima (97%+) → EXPERT

---

## 📥 Download Size Reference

```
MINIMAL Profile:
  - phi-2: 1.4GB
  - all-MiniLM-L6-v2: 22MB
  - PaddleOCR: 100MB
  - Dependencies: ~400MB
  Total: ~2GB download, ~5GB extracted

STANDARD Profile:
  - mistral-7b: 4GB
  - all-mpnet-base-v2: 438MB
  - PaddleOCR: 120MB
  - Dependencies: ~500MB
  Total: ~5GB download, ~12GB extracted

ADVANCED Profile:
  - neural-chat-7b: 4GB
  - mistral-7b: 4GB (backup)
  - bge-large-en: 1.3GB
  - PaddleOCR: 200MB
  Total: ~10GB download, ~25GB extracted

EXPERT Profile:
  - llama-2-70b: 39GB
  - mistral-7b: 4GB (fallback)
  - instructor-large: 1.5GB
  Total: ~50GB download, ~80GB extracted
```

---

## ⏱️ Tempi Setup Stimati

(con connessione 80Mbps)

| Profilo | Download | Installation | Total |
|---------|----------|--------------|-------|
| MINIMAL | 5 min | 5 min | 10 min |
| STANDARD | 15 min | 10 min | 25 min |
| ADVANCED | 30 min | 10 min | 40 min |
| EXPERT | 120 min | 15 min | 135 min |

---

## 🚀 Prossimi Step

Nel `setup.sh` aggiungeremo:
1. Menu di selezione profili
2. Download parallelo dei modelli
3. Validazione e cache
4. Configurazione automatica variabili
5. Health check per ogni profilo
