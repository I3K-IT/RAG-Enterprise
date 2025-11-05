# GUIDA ALLE MODIFICHE ESATTE - DIFF PER FILE

Questa guida mostra ESATTAMENTE cosa è cambiato in ogni file.

---

## 📄 FILE: ocr_service.py

### CAMBIO #1: Aggiungi MIME_TYPES dict (dopo import)

**Dove**: Dopo le importazioni, prima della classe

```python
# AGGIUNGI:
class OCRService:
    """
    Servizio OCR universale...
    """
    
    TIKA_URL = "http://localhost:9998/tika"
    
    # ✅ NUOVO: Mapping MIME types
    MIME_TYPES = {
        '.pdf': 'application/pdf',
        '.txt': 'text/plain',
        '.docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
        '.doc': 'application/msword',
        # ... (vedi file completo nel output)
    }
```

### CAMBIO #2: Modifica __init__ per supportare PaddleOCR

**Prima**:
```python
def __init__(self):
    logger.info("Inizializzando OCR Service (Tika)...")
    
    try:
        self.tika_process = subprocess.Popen(...)
        # ... resto del codice
```

**Dopo**:
```python
def __init__(self):
    logger.info("Inizializzando OCR Service...")
    
    self.tika_ready = False
    self.paddle_ready = False
    self.paddle_ocr = None
    
    try:
        self._start_tika()
        self.tika_ready = True
    except Exception as e:
        logger.warning(f"⚠️  Tika non disponibile: {str(e)}")
    
    try:
        self._init_paddle_ocr()
        self.paddle_ready = True
    except Exception as e:
        logger.warning(f"⚠️  PaddleOCR non disponibile: {str(e)}")
    
    if not self.tika_ready and not self.paddle_ready:
        raise RuntimeError("No OCR backend available")
```

### CAMBIO #3: Estrai _start_tika() da __init__

**Nuovo metodo**:
```python
def _start_tika(self):
    logger.info("Avviando Tika server...")
    
    self.tika_process = subprocess.Popen(...)
    # ... resto del codice
```

### CAMBIO #4: Aggiungi _init_paddle_ocr()

**Nuovo metodo**:
```python
def _init_paddle_ocr(self):
    from paddleocr import PaddleOCR
    logger.info("Caricando PaddleOCR...")
    self.paddle_ocr = PaddleOCR(use_angle_cls=True, lang='it')
    logger.info("✅ PaddleOCR pronto")
```

### CAMBIO #5: Aggiungi _get_mime_type()

**Nuovo metodo**:
```python
def _get_mime_type(self, file_path: str) -> str:
    ext = Path(file_path).suffix.lower()
    mime_type = self.MIME_TYPES.get(ext, 'application/octet-stream')
    logger.debug(f"  MIME type per {ext}: {mime_type}")
    return mime_type
```

### CAMBIO #6: Modifica extract_text()

**Prima**:
```python
def extract_text(self, file_path: str) -> str:
    try:
        logger.info(f"Estraendo testo da: {file_path}")
        
        with open(file_path, 'rb') as f:
            file_data = f.read()
        
        response = requests.put(
            self.TIKA_URL,
            data=file_data,
            headers={'Content-Type': 'text/plain'},  # ❌ Hard-coded!
            timeout=30
        )
        
        if response.status_code == 200:
            text = self._extract_text_from_tika_xml(response.text)
            logger.info(f"✅ Estratti {len(text)} caratteri")
            return text
        else:
            logger.error(f"❌ Errore Tika: {response.status_code}")
            return ""
    
    except Exception as e:
        logger.error(f"❌ Errore estrazione testo: {str(e)}")
        return ""
```

**Dopo**:
```python
def extract_text(self, file_path: str) -> str:
    try:
        logger.info(f"📄 Estraendo testo da: {Path(file_path).name}")
        
        # Prova Tika first
        if self.tika_ready:
            logger.debug("  Tentando Tika...")
            text = self._extract_with_tika(file_path)
            
            if text and len(text.strip()) > 0:
                logger.info(f"✅ Estratti {len(text)} caratteri con Tika")
                return text
            else:
                logger.warning("  ⚠️  Tika ritornò testo vuoto, provando PaddleOCR...")
        
        # Fallback a PaddleOCR
        if self.paddle_ready:
            logger.debug("  Tentando PaddleOCR...")
            text = self._extract_with_paddle_ocr(file_path)
            
            if text and len(text.strip()) > 0:
                logger.info(f"✅ Estratti {len(text)} caratteri con PaddleOCR")
                return text
        
        logger.error("❌ Nessun metodo OCR ha estratto testo!")
        return ""
    
    except Exception as e:
        logger.error(f"❌ Errore estrazione testo: {str(e)}")
        return ""
```

### CAMBIO #7: Aggiungi _extract_with_tika()

**Nuovo metodo**:
```python
def _extract_with_tika(self, file_path: str) -> str:
    try:
        with open(file_path, 'rb') as f:
            file_data = f.read()
        
        mime_type = self._get_mime_type(file_path)  # ✅ Dynamic!
        
        response = requests.put(
            self.TIKA_URL,
            data=file_data,
            headers={
                'Content-Type': mime_type,  # ✅ Correct MIME type!
                'Accept': 'application/xml'
            },
            timeout=30
        )
        
        if response.status_code == 200:
            text = self._extract_text_from_tika_xml(response.text)
            return text
        else:
            logger.warning(f"  Tika status: {response.status_code}")
            return ""
    
    except Exception as e:
        logger.warning(f"  Errore Tika: {str(e)}")
        return ""
```

### CAMBIO #8: Aggiungi _extract_with_paddle_ocr()

**Nuovo metodo**:
```python
def _extract_with_paddle_ocr(self, file_path: str) -> str:
    try:
        result = self.paddle_ocr.ocr(file_path, cls=True)
        
        if not result:
            return ""
        
        text_lines = []
        for line in result:
            for word_info in line:
                text = word_info[1]
                text_lines.append(text)
        
        return "\n".join(text_lines)
    
    except Exception as e:
        logger.warning(f"  Errore PaddleOCR: {str(e)}")
        return ""
```

### CAMBIO #9: Migliora _extract_text_from_tika_xml()

**Prima**:
```python
def _extract_text_from_tika_xml(self, xml_text: str) -> str:
    try:
        root = ET.fromstring(xml_text)
        ns = {'xhtml': 'http://www.w3.org/1999/xhtml'}
        body = root.find('.//xhtml:body', ns)
        if body is not None:
            text = ''.join(body.itertext())
            return text.strip()
        
        return ""
    
    except Exception as e:
        logger.warning(f"⚠️  Errore parsing XML Tika: {str(e)}")
        return ""
```

**Dopo**:
```python
def _extract_text_from_tika_xml(self, xml_text: str) -> str:
    try:
        # Rimuovi BOM se presente
        if xml_text.startswith('\ufeff'):
            xml_text = xml_text[1:]
        
        root = ET.fromstring(xml_text)
        ns = {'xhtml': 'http://www.w3.org/1999/xhtml'}
        
        body = root.find('.//xhtml:body', ns)
        if body is not None:
            text = ''.join(body.itertext())
            return text.strip()
        
        # Fallback senza namespace
        body = root.find('.//body')
        if body is not None:
            text = ''.join(body.itertext())
            return text.strip()
        
        # Ultimo resort
        text = ''.join(root.itertext())
        return text.strip()
    
    except Exception as e:
        logger.warning(f"  Errore parsing XML Tika: {str(e)}")
        return xml_text.strip() if xml_text else ""
```

---

## 📄 FILE: rag_pipeline.py

### CAMBIO #1: Aggiungi __init__ parameter

**Prima**:
```python
def __init__(
    self,
    qdrant_connector,
    embeddings_service,
    llm_model: str = "mistral",
    chunk_size: int = 500,
    chunk_overlap: int = 100
):
```

**Dopo**:
```python
def __init__(
    self,
    qdrant_connector,
    embeddings_service,
    llm_model: str = "mistral",
    chunk_size: int = 500,
    chunk_overlap: int = 100,
    relevance_threshold: float = 0.5  # ✅ NUOVO
):
    # ... existing code ...
    self.relevance_threshold = relevance_threshold  # ✅ Save it
```

### CAMBIO #2: Migliora chunk_text() logging

**Prima**:
```python
def chunk_text(self, text: str, ...):
    # ... splitting ...
    chunks = splitter.split_text(text)
    logger.info(f"Text chunked in {len(chunks)} pieces")
    return chunks
```

**Dopo**:
```python
def chunk_text(self, text: str, ...):
    # ... splitting ...
    chunks = splitter.split_text(text)
    logger.info(f"📊 Text suddiviso in {len(chunks)} chunks (size={chunk_size}, overlap={overlap})")
    return chunks
```

### CAMBIO #3: Migliora index_chunks()

**Prima**:
```python
def index_chunks(self, chunks: List[str], document_id: str, filename: str):
    try:
        logger.info(f"Indexing {len(chunks)} chunks per {filename}")
        
        embeddings = self.embeddings_service.embed_texts(chunks)
        logger.info(f"Generated {len(embeddings)} embeddings")
        
        self.qdrant_connector.insert_vectors(...)
        logger.info(f"✅ Indexing completato per {filename}")
```

**Dopo**:
```python
def index_chunks(self, chunks: List[str], document_id: str, filename: str):
    try:
        if not chunks:
            logger.warning(f"⚠️  Nessun chunk da indicizzare per {filename}")
            return
        
        logger.info(f"📇 Indicizzando {len(chunks)} chunks per '{filename}'")
        
        logger.debug(f"  1/2 Generando embeddings...")
        embeddings = self.embeddings_service.embed_texts(chunks)
        
        if not embeddings:
            logger.error(f"❌ Embedding service ha ritornato lista vuota!")
            return
        
        logger.info(f"      ✅ {len(embeddings)} embeddings generati")
        
        # ... rest of code ...
        logger.info(f"✅ Indexing completato per '{filename}' ({len(chunks)} chunks)")
```

### CAMBIO #4: Completa rewrite di query()

**Prima**:
```python
def query(self, query: str, top_k: int = 5, temperature: float = 0.7) -> Tuple[str, List[dict]]:
    try:
        logger.info(f"RAG Query: '{query}' (top_k={top_k})")
        
        query_embedding = self.embeddings_service.embed_text(query)
        retrieved_docs = self.qdrant_connector.search(
            query_vector=query_embedding,
            top_k=top_k
        )
        
        logger.info(f"      ✅ Recuperati {len(retrieved_docs)} documenti")
        
        context = "\n\n".join([
            doc["metadata"]["text"] for doc in retrieved_docs
        ])
        
        prompt = self.qa_prompt.format(
            context=context,
            question=query
        )
        
        answer = self.llm(prompt)
        logger.info(f"      ✅ Risposta generata ({len(answer)} caratteri)")
        
        sources = [
            {
                "filename": doc["metadata"]["filename"],
                "document_id": doc["metadata"]["document_id"],
                "chunk_index": doc["metadata"]["chunk_index"],
                "similarity_score": doc.get("similarity", 0)
            }
            for doc in retrieved_docs
        ]
        
        return answer, sources
```

**Dopo** (vedi file completo nell'output per la versione completa, punti chiave):
```python
def query(self, query: str, top_k: int = 5, temperature: float = 0.7) -> Tuple[str, List[Dict]]:
    try:
        logger.info(f"❓ RAG Query: '{query}' (top_k={top_k}, threshold={self.relevance_threshold})")
        
        # ... retrieval ...
        
        # ✅ NUOVO: Filtra per relevance threshold
        relevant_docs = [
            doc for doc in retrieved_docs
            if doc.get("similarity", 0) >= self.relevance_threshold
        ]
        
        logger.info(f"      ✅ {len(relevant_docs)}/{len(retrieved_docs)} documenti sopra threshold")
        
        if not relevant_docs:
            logger.warning(f"⚠️  Nessun documento supera il threshold {self.relevance_threshold}")
            return "Non ho trovato informazioni sufficientemente rilevanti per rispondere.", []
        
        # ... LLM generation ...
        
        # ✅ NUOVO: Deduplicazione sources
        sources_dict = {}
        for doc in relevant_docs:
            doc_id = doc["metadata"].get("document_id", "unknown")
            similarity = doc.get("similarity", 0)
            
            if doc_id not in sources_dict or similarity > sources_dict[doc_id]["similarity_score"]:
                sources_dict[doc_id] = {
                    "filename": doc["metadata"]["filename"],
                    "document_id": doc_id,
                    "similarity_score": round(similarity, 3),
                    "chunk_index": doc["metadata"]["chunk_index"],
                }
        
        sources = list(sources_dict.values())
        sources.sort(key=lambda x: x["similarity_score"], reverse=True)
        
        logger.info(f"✅ Query completata - {len(sources)} sources uniche ritornate")
        
        return answer, sources
```

---

## 📄 FILE: app.py

### CAMBIO #1: Aggiungi import per RELEVANCE_THRESHOLD

**Dopo le dichiarazioni di env vars**:
```python
UPLOAD_DIR = os.getenv("UPLOAD_DIR", "./uploads")
CUDA_VISIBLE_DEVICES = os.getenv("CUDA_VISIBLE_DEVICES", "0")
RELEVANCE_THRESHOLD = float(os.getenv("RELEVANCE_THRESHOLD", "0.5"))  # ✅ NUOVO
```

### CAMBIO #2: Migliora logger setup

**Prima**:
```python
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)
```

**Dopo**:
```python
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'  # ✅ NUOVO
)
logger = logging.getLogger(__name__)
```

### CAMBIO #3: Migliora startup_event() logging

**Prima**:
```python
@app.on_event("startup")
async def startup_event():
    global ocr_service, embeddings_service, rag_pipeline, qdrant_connector
    
    logger.info("🚀 Inizializzazione RAG Backend...")
```

**Dopo**:
```python
@app.on_event("startup")
async def startup_event():
    global ocr_service, embeddings_service, rag_pipeline, qdrant_connector
    
    logger.info("=" * 80)
    logger.info("🚀 AVVIO RAG BACKEND")
    logger.info("=" * 80)
    logger.info(f"Configurazione:")
    logger.info(f"  - QDRANT: {QDRANT_HOST}:{QDRANT_PORT}")
    logger.info(f"  - LLM: {LLM_MODEL}")
    logger.info(f"  - Embedding: {EMBEDDING_MODEL}")
    logger.info(f"  - Relevance Threshold: {RELEVANCE_THRESHOLD}")  # ✅ NUOVO
    logger.info(f"  - Upload Dir: {UPLOAD_DIR}")
    logger.info(f"  - CUDA Devices: {CUDA_VISIBLE_DEVICES}")
    logger.info("=" * 80)
    
    # ... rest of code ...
```

### CAMBIO #4: Aggiungi SourceInfo model

**Dopo la classe QueryRequest**:
```python
# ✅ NUOVO
class SourceInfo(BaseModel):
    filename: str
    document_id: str
    similarity_score: float
    chunk_index: Optional[int] = None
```

### CAMBIO #5: Aggiorna QueryResponse

**Prima**:
```python
class QueryResponse(BaseModel):
    answer: str
    sources: List[dict]  # ❌ Non tipizzato
    processing_time: float
```

**Dopo**:
```python
class QueryResponse(BaseModel):
    answer: str
    sources: List[SourceInfo]  # ✅ Tipizzato!
    processing_time: float
    num_sources: int  # ✅ NUOVO
```

### CAMBIO #6: Migliora HealthResponse

**Prima**:
```python
class HealthResponse(BaseModel):
    status: str
    backend_version: str
    qdrant_connected: bool
    services: dict
```

**Dopo**:
```python
class HealthResponse(BaseModel):
    status: str
    backend_version: str
    qdrant_connected: bool
    services: dict
    configuration: dict  # ✅ NUOVO
```

### CAMBIO #7: Migliora RAGPipeline init

**Prima**:
```python
rag_pipeline = RAGPipeline(
    qdrant_connector=qdrant_connector,
    embeddings_service=embeddings_service,
    llm_model=LLM_MODEL
)
```

**Dopo**:
```python
rag_pipeline = RAGPipeline(
    qdrant_connector=qdrant_connector,
    embeddings_service=embeddings_service,
    llm_model=LLM_MODEL,
    relevance_threshold=RELEVANCE_THRESHOLD  # ✅ Passa il threshold!
)
```

### CAMBIO #8: Migliora process_document_background()

**Aggiungi logging dettagliato con separatori e timing**:
```python
async def process_document_background(file_path: str, document_id: str, filename: str):
    logger.info("=" * 80)
    logger.info(f"📇 INIZIO PROCESSING: {filename}")
    logger.info(f"   Document ID: {document_id}")
    logger.info("=" * 80)
    
    try:
        # STEP 1: OCR Extraction
        logger.info(f"  [1/3] OCR Extraction...")
        start_ocr = datetime.now()
        
        text = ocr_service.extract_text(file_path)
        
        ocr_time = (datetime.now() - start_ocr).total_seconds()
        logger.info(f"        ✅ Estratti {len(text)} caratteri in {ocr_time:.2f}s")
        
        if not text or len(text.strip()) == 0:
            logger.warning(f"⚠️  ATTENZIONE: OCR ha ritornato testo vuoto!")
        
        # STEP 2: Chunking
        logger.info(f"  [2/3] Document Chunking...")
        start_chunk = datetime.now()
        
        chunks = rag_pipeline.chunk_text(text, chunk_size=500, overlap=100)
        
        chunk_time = (datetime.now() - start_chunk).total_seconds()
        logger.info(f"        ✅ {len(chunks)} chunks creati in {chunk_time:.2f}s")
        
        if not chunks:
            logger.error(f"❌ ERRORE: Nessun chunk creato!")
            return
        
        # STEP 3: Embedding & Indexing
        logger.info(f"  [3/3] Embedding & Indexing...")
        start_index = datetime.now()
        
        rag_pipeline.index_chunks(
            chunks=chunks,
            document_id=document_id,
            filename=filename
        )
        
        index_time = (datetime.now() - start_index).total_seconds()
        logger.info(f"        ✅ Indicizzato su Qdrant in {index_time:.2f}s")
        
        # SUMMARY
        total_time = (datetime.now() - start_ocr).total_seconds()
        logger.info("=" * 80)
        logger.info(f"✅ PROCESSING COMPLETATO: {filename}")
        logger.info(f"   Tempo totale: {total_time:.2f}s")
        logger.info(f"   - OCR: {ocr_time:.2f}s")
        logger.info(f"   - Chunking: {chunk_time:.2f}s")
        logger.info(f"   - Indexing: {index_time:.2f}s")
        logger.info(f"   Chunks: {len(chunks)}")
        logger.info(f"   Caratteri: {len(text)}")
        logger.info("=" * 80)
        
    except Exception as e:
        logger.error("=" * 80)
        logger.error(f"❌ ERRORE PROCESSING {filename}: {str(e)}")
        logger.error(traceback.format_exc())  # ✅ Stack trace!
        logger.error("=" * 80)
```

### CAMBIO #9: Migliora query_rag() response

**Prima**:
```python
return QueryResponse(
    answer=answer,
    sources=sources,
    processing_time=processing_time
)
```

**Dopo**:
```python
return QueryResponse(
    answer=answer,
    sources=[SourceInfo(**src) for src in sources],  # ✅ Type conversion
    processing_time=processing_time,
    num_sources=len(sources)  # ✅ NUOVO
)
```

---

## 📋 SUMMARY DELLE MODIFICHE

| File | Cambamenti | Righe |
|------|-----------|-------|
| ocr_service.py | MIME type dinamico + PaddleOCR fallback | +150 |
| rag_pipeline.py | Filtering + deduplication | +40 |
| app.py | Logging dettagliato + typing | +60 |
| **TOTALE** | **Miglioramenti completi** | **+250 righe** |

---

**Nota**: I file completi sono disponibili in `/mnt/user-data/outputs/`
**Consiglio**: Copia i file interi dal output folder, non fare copy-paste manuale!