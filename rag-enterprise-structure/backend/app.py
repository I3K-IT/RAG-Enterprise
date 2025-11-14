"""
RAG Enterprise Backend - FastAPI Application
Gestisce: OCR, Embedding, RAG Pipeline, Qdrant Integration
"""

from fastapi import FastAPI, File, UploadFile, HTTPException, BackgroundTasks
from fastapi.responses import JSONResponse
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
from typing import List, Optional
import os
import logging
from datetime import datetime
import traceback

from rag_pipeline import RAGPipeline
from ocr_service import OCRService
from embeddings_service import EmbeddingsService
from qdrant_connector import QdrantConnector
import re
from typing import Dict, Optional

def detect_document_type(text: str) -> str:
    """Riconosce il tipo di documento - con check più rigorosi"""
    text_upper = text.upper()
    
    # Ordine: più specifico → meno specifico
    
    # 1. IDENTITY CARD (molto specifico)
    if 'CARTA DI IDENTITA' in text_upper or 'IDENTITY CARD' in text_upper:
        if 'REPUBBLICA ITALIANA' in text_upper:  # Extra check
            return 'IDENTITY_CARD'
    
    # 2. PASSAPORTO (molto specifico)
    if 'PASSAPORTO' in text_upper or 'PASSPORT' in text_upper:
        if 'REPUBBLICA ITALIANA' in text_upper:
            return 'PASSPORT'
    
    # 3. PATENTE (molto specifico)
    if 'PATENTE DI GUIDA' in text_upper or 'DRIVING LICENSE' in text_upper:
        return 'DRIVING_LICENSE'
    
    # 4. CONTRATTO
    if 'CONTRATTO' in text_upper or 'CONTRACT' in text_upper or 'AGREEMENT' in text_upper:
        return 'CONTRACT'
    
    # DEFAULT
    return 'GENERIC_DOCUMENT'


def extract_id_fields(text: str) -> Dict[str, Optional[str]]:
    """Estrae campi da Carta d'Identità - layout verticale"""
    fields = {}
    
    # Codice Fiscale: dopo "CODICE FISCALE" o "FISCAL CODE", sulla riga successiva
    # Pattern: 16 caratteri esatti (6 lettere + 2 digit + 1 lettera + 2 digit + 1 lettera + 3 digit + 1 lettera)
    cf_pattern = r'(?:CODICE\s+FISCALE|FISCAL\s+CODE)\s*\n\s*([A-Z]{6}\d{2}[A-Z]\d{2}[A-Z]\d{3}[A-Z])'
    cf_match = re.search(cf_pattern, text, re.IGNORECASE | re.MULTILINE)
    
    if cf_match:
        fields['codice_fiscale'] = cf_match.group(1)
    
    # Indirizzo
    addr_pattern = r'(VIA|VIALE|PIAZZA|CORSO|STRADA)\s+([A-Z\s,\'-]+?),\s+N\.\s+(\d+)\s+([A-Z\s\(\)]+)'
    addr_match = re.search(addr_pattern, text)
    if addr_match:
        fields['indirizzo'] = f"{addr_match.group(1)} {addr_match.group(2)}, N. {addr_match.group(3)} {addr_match.group(4)}"
    
    # Data nascita (cerca dopo "LUOGO E DATA DI NASCITA")
    date_pattern = r'(?:LUOGO\s+E\s+DATA|PLACE\s+AND\s+DATE)[^\n]*\n\s*([A-Z\s]+)\s+(\d{1,2})[./](\d{1,2})[./](\d{4})'
    date_match = re.search(date_pattern, text, re.IGNORECASE | re.MULTILINE)
    if date_match:
        fields['data_nascita'] = f"{date_match.group(2)}.{date_match.group(3)}.{date_match.group(4)}"
        fields['luogo_nascita'] = date_match.group(1).strip()
    
    return fields


def extract_passport_fields(text: str) -> Dict[str, Optional[str]]:
    """Estrae campi da Passaporto"""
    fields = {}
    
    # Numero passaporto (di solito 9 caratteri)
    passport_pattern = r'[A-Z]{2}\d{7}'
    passport_match = re.search(passport_pattern, text)
    if passport_match:
        fields['numero_passaporto'] = passport_match.group()
    
    return fields


def extract_license_fields(text: str) -> Dict[str, Optional[str]]:
    """Estrae campi da Patente - CON check rigorosi"""
    fields = {}
    
    # Check 1: deve contenere "PATENTE DI GUIDA"
    if 'PATENTE DI GUIDA' not in text.upper() and 'DRIVING LICENSE' not in text.upper():
        return fields
    
    # Check 2: pattern numero patente italiano (10 caratteri alphanumerici)
    # Ma SOLO se preceduto da specifiche keywords
    license_pattern = r'(?:Numero|Number|N\.|Nr\.)\s*[:\s]*([A-Z0-9]{10})'
    license_match = re.search(license_pattern, text)
    if license_match:
        fields['numero_patente'] = license_match.group(1)
    
    return fields


def extract_structured_fields(text: str, doc_type: str) -> Dict[str, Optional[str]]:
    """Estrae campi strutturati in base al tipo di documento"""
    
    if doc_type == 'IDENTITY_CARD':
        return extract_id_fields(text)
    elif doc_type == 'PASSPORT':
        return extract_passport_fields(text)
 #   elif doc_type == 'DRIVING_LICENSE':
 #       return extract_license_fields(text)
    else:
        return {}

# Logging setup - PIU' DETTAGLIATO
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

# Environment variables
QDRANT_HOST = os.getenv("QDRANT_HOST", "qdrant")
QDRANT_PORT = int(os.getenv("QDRANT_PORT", 6333))
LLM_MODEL = os.getenv("LLM_MODEL", "mistral")
EMBEDDING_MODEL = os.getenv("EMBEDDING_MODEL", "all-MiniLM-L6-v2")
UPLOAD_DIR = os.getenv("UPLOAD_DIR", "./uploads")
CUDA_VISIBLE_DEVICES = os.getenv("CUDA_VISIBLE_DEVICES", "0")
RELEVANCE_THRESHOLD = float(os.getenv("RELEVANCE_THRESHOLD", "0.3"))

# Crea cartella upload se non esiste
os.makedirs(UPLOAD_DIR, exist_ok=True)

# FastAPI App
app = FastAPI(
    title="RAG Enterprise Backend",
    description="API per RAG Pipeline Distribuito",
    version="1.0.0"
)

# CORS configuration
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Servizi globali
ocr_service: Optional[OCRService] = None
embeddings_service: Optional[EmbeddingsService] = None
rag_pipeline: Optional[RAGPipeline] = None
qdrant_connector: Optional[QdrantConnector] = None

# Memoria conversazionale per utenti
user_conversations: dict = {}  # {user_id: [{"user": "...", "assistant": "..."}]}


# ============================================================================
# INITIALIZATION
# ============================================================================

@app.on_event("startup")
async def startup_event():
    """Inizializza i servizi al startup"""
    global ocr_service, embeddings_service, rag_pipeline, qdrant_connector
    
    logger.info("=" * 80)
    logger.info("🚀 AVVIO RAG BACKEND")
    logger.info("=" * 80)
    logger.info(f"Configurazione:")
    logger.info(f"  - QDRANT: {QDRANT_HOST}:{QDRANT_PORT}")
    logger.info(f"  - LLM: {LLM_MODEL}")
    logger.info(f"  - Embedding: {EMBEDDING_MODEL}")
    logger.info(f"  - Relevance Threshold: {RELEVANCE_THRESHOLD}")
    logger.info(f"  - Upload Dir: {UPLOAD_DIR}")
    logger.info(f"  - CUDA Devices: {CUDA_VISIBLE_DEVICES}")
    logger.info("=" * 80)
    
    try:
        # 1. Connessione Qdrant
        logger.info("🔗 [1/4] Connessione a Qdrant...")
        qdrant_connector = QdrantConnector(
            host="qdrant",
            port=6333
        )
        qdrant_connector.connect()
        logger.info("✅ Qdrant connesso")
        
        # 2. Servizio OCR
        logger.info("🔗 [2/4] Caricamento OCR Service...")
        try:
            ocr_service = OCRService()
            logger.info("✅ OCR Service pronto")
        except Exception as e:
            logger.warning(f"⚠️  OCR Service fallito: {e}")
            logger.warning("    → Sistema continuerà senza OCR")
            ocr_service = None

        # 3. Servizio Embedding
        logger.info(f"🔗 [3/4] Caricamento Embedding Service ({EMBEDDING_MODEL})...")
        embeddings_service = EmbeddingsService(model_name=EMBEDDING_MODEL)
        logger.info("✅ Embedding Service pronto")
        
        # 4. Pipeline RAG
        logger.info(f"🔗 [4/4] Inizializzazione RAG Pipeline (LLM: {LLM_MODEL})...")
        rag_pipeline = RAGPipeline(
            qdrant_connector=qdrant_connector,
            embeddings_service=embeddings_service,
            llm_model=LLM_MODEL,
            relevance_threshold=RELEVANCE_THRESHOLD
        )
        logger.info("✅ RAG Pipeline pronto")
        
        logger.info("=" * 80)
        logger.info("🎉 BACKEND COMPLETAMENTE INIZIALIZZATO")
        logger.info("=" * 80)
        
    except Exception as e:
        logger.error("=" * 80)
        logger.error(f"❌ ERRORE DURANTE STARTUP: {str(e)}")
        logger.error(traceback.format_exc())
        logger.error("=" * 80)
        raise


@app.on_event("shutdown")
async def shutdown_event():
    """Cleanup al shutdown"""
    logger.info("🛑 Shutdown RAG Backend...")
    if qdrant_connector:
        qdrant_connector.disconnect()
    logger.info("✅ Cleanup completato")


# ============================================================================
# PYDANTIC MODELS
# ============================================================================

class QueryRequest(BaseModel):
    query: str
    top_k: int = 5
    temperature: float = 0.7


class SourceInfo(BaseModel):
    filename: str
    document_id: str
    similarity_score: float
    chunk_index: Optional[int] = None
    text: Optional[str] = None 


class QueryResponse(BaseModel):
    answer: str
    sources: List[SourceInfo]
    processing_time: float
    num_sources: int


class DocumentMetadata(BaseModel):
    filename: str
    upload_date: str
    document_id: str
    num_chunks: int
    status: str


class DocumentsListResponse(BaseModel):
    documents: List[DocumentMetadata]
    total: int


class HealthResponse(BaseModel):
    status: str
    backend_version: str
    qdrant_connected: bool
    services: dict
    configuration: dict


# ============================================================================
# HEALTH & INFO ENDPOINTS
# ============================================================================

@app.get("/health", response_model=HealthResponse)
async def health_check():
    """Health check endpoint"""
    all_ready = all([ocr_service, embeddings_service, rag_pipeline, qdrant_connector])
    
    return HealthResponse(
        status="healthy" if all_ready else "degraded",
        backend_version="1.0.0",
        qdrant_connected=qdrant_connector.is_connected() if qdrant_connector else False,
        services={
            "ocr": ocr_service is not None,
            "embeddings": embeddings_service is not None,
            "rag_pipeline": rag_pipeline is not None,
            "qdrant": qdrant_connector is not None
        },
        configuration={
            "llm_model": LLM_MODEL,
            "embedding_model": EMBEDDING_MODEL,
            "relevance_threshold": RELEVANCE_THRESHOLD,
        }
    )


@app.get("/info")
async def get_info():
    """Informazioni configurazione"""
    return {
        "backend": "RAG Enterprise v1.0.0",
        "qdrant": f"{QDRANT_HOST}:{QDRANT_PORT}",
        "llm_model": LLM_MODEL,
        "embedding_model": EMBEDDING_MODEL,
        "cuda_devices": CUDA_VISIBLE_DEVICES,
        "relevance_threshold": RELEVANCE_THRESHOLD,
    }


# ============================================================================
# DOCUMENT MANAGEMENT
# ============================================================================

# Formati supportati
ALLOWED_EXTENSIONS = {
    '.pdf', '.txt', '.doc', '.docx', '.ppt', '.pptx', '.xls', '.xlsx', 
    '.odt', '.rtf', '.html', '.xml', '.json', '.csv', '.md',
    '.jpg', '.jpeg', '.png', '.gif', '.bmp'
}

@app.post("/api/documents/upload")
async def upload_document(
    file: UploadFile = File(...),
    background_tasks: BackgroundTasks = None
):
    """
    Carica un documento (qualsiasi formato) e lo processa in background
    Formati supportati: PDF, DOCX, PPTX, XLSX, ODT, RTF, HTML, XML, JSON, CSV, Immagini
    """
    
    if not ocr_service or not embeddings_service or not rag_pipeline:
        raise HTTPException(
            status_code=503, 
            detail="Servizi non inizializzati. Controlla /health"
        )
    
    # Controlla estensione file
    from pathlib import Path
    file_ext = Path(file.filename).suffix.lower()
    
    if file_ext not in ALLOWED_EXTENSIONS:
        raise HTTPException(
            status_code=400, 
            detail=f"Formato '{file_ext}' non supportato. Supportati: {', '.join(sorted(ALLOWED_EXTENSIONS))}"
        )
    
    try:
        # Crea document_id con timestamp PRIMA
        document_id = f"{datetime.now().timestamp()}_{file.filename}"
        file_path = os.path.join(UPLOAD_DIR, document_id)
        content = await file.read()
        
        # Salva file con il document_id (con timestamp)
        with open(file_path, "wb") as f:
            f.write(content)
        
        logger.info(f"📄 File ricevuto: '{file.filename}' ({len(content)} bytes)")
        logger.info(f"   Document ID: {document_id}")
        logger.info(f"   File path: {file_path}")
        
        # Aggiungi background task
        background_tasks.add_task(
            process_document_background,
            file_path,
            document_id,
            file.filename
        )
        
        return JSONResponse(
            status_code=202,
            content={
                "message": "Documento ricevuto, elaborazione in corso",
                "document_id": document_id,
                "filename": file.filename,
                "size_bytes": len(content)
            }
        )
        
    except Exception as e:
        logger.error(f"❌ Errore upload: {str(e)}")
        logger.error(traceback.format_exc())
        raise HTTPException(status_code=500, detail=str(e))


async def process_document_background(file_path: str, document_id: str, filename: str):
    """Background task per processare documento - LOGGING DETTAGLIATO"""
    try:
        logger.info("=" * 80)
        logger.info(f"📇 INIZIO PROCESSING: {filename}")
        logger.info(f"   Document ID: {document_id}")
        logger.info(f"   File path: {file_path}")
        logger.info("=" * 80)
        
        # STEP 1: OCR Extraction
        logger.info(f"  [1/3] OCR Extraction...")
        start_ocr = datetime.now()
        
        try:
            text = ocr_service.extract_text(file_path)

            # NUOVO: Detect document type e estrai campi strutturati
            doc_type = detect_document_type(text)
            structured_fields = extract_structured_fields(text, doc_type)

            logger.info(f"Document Type: {doc_type}")
            logger.info(f"Structured Fields: {structured_fields}")
        except Exception as e:
            logger.error(f"      ❌ OCR FAILED: {str(e)}", exc_info=True)
            text = ""
        
        ocr_time = (datetime.now() - start_ocr).total_seconds()
        logger.info(f"        ✅ Estratti {len(text)} caratteri in {ocr_time:.2f}s")
        
        if not text or len(text.strip()) == 0:
            logger.warning(f"⚠️  ATTENZIONE: OCR ha ritornato testo vuoto!")
            return
        
        # STEP 2: Chunking
        logger.info(f"  [2/3] Document Chunking...")
        start_chunk = datetime.now()
        
        try:
            chunks = rag_pipeline.chunk_text(text, chunk_size=1000, overlap=100)
        except Exception as e:
            logger.error(f"      ❌ CHUNKING FAILED: {str(e)}", exc_info=True)
            return
        
        chunk_time = (datetime.now() - start_chunk).total_seconds()
        logger.info(f"        ✅ {len(chunks)} chunks creati in {chunk_time:.2f}s")
        
        if not chunks:
            logger.error(f"❌ ERRORE: Nessun chunk creato!")
            return
        
        # STEP 3: Embedding & Indexing
        logger.info(f"  [3/3] Embedding & Indexing...")
        start_index = datetime.now()
        
        try:
            rag_pipeline.index_chunks(
                chunks=chunks,
                document_id=document_id,
                filename=filename,
                document_type=doc_type,
                structured_fields=structured_fields                
            )
        except Exception as e:
            logger.error(f"      ❌ INDEXING FAILED: {str(e)}", exc_info=True)
            return
        
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
        logger.error(f"❌ ERRORE CRITICO PROCESSING {filename}: {str(e)}")
        logger.error(traceback.format_exc())
        logger.error("=" * 80)


@app.get("/api/documents", response_model=DocumentsListResponse)
async def list_documents():
    """Lista documenti indicizzati"""
    if not qdrant_connector:
        raise HTTPException(status_code=503, detail="Qdrant non connesso")
    
    try:
        docs = qdrant_connector.get_indexed_documents()
        return DocumentsListResponse(
            documents=docs,
            total=len(docs)
        )
    except Exception as e:
        logger.error(f"Errore lista documenti: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/api/documents/{document_id}/download")
async def download_document(document_id: str):
    """Scarica il documento originale caricato"""
    from fastapi.responses import FileResponse
    import glob
    
    try:
        # Cerca il file nella cartella upload
        # Il document_id ha formato: timestamp_nomefile.ext
        search_pattern = os.path.join(UPLOAD_DIR, f"{document_id}")
        
        # Verifica se il file esiste esattamente
        if os.path.exists(search_pattern):
            file_path = search_pattern
        else:
            # Altrimenti cerca con wildcard (nel caso il path sia leggermente diverso)
            files = glob.glob(search_pattern)
            if not files:
                raise HTTPException(status_code=404, detail=f"Documento non trovato: {document_id}")
            file_path = files[0]
        
        logger.info(f"📥 Download documento: {document_id}")
        logger.info(f"   Path: {file_path}")
        
        if not os.path.exists(file_path):
            raise HTTPException(status_code=404, detail="File non trovato")
        
        # Estrai il nome del file originale (senza timestamp)
        # Formato: 1762533561.231156_TU-81-08-Ed.-Gennaio-2025-1.pdf
        filename = os.path.basename(file_path)
        original_filename = filename.split('_', 1)[1] if '_' in filename else filename
        
        return FileResponse(
            path=file_path,
            media_type='application/octet-stream',
            filename=original_filename
        )
    
    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"❌ Errore download: {str(e)}")
        logger.error(traceback.format_exc())
        raise HTTPException(status_code=500, detail=str(e))


@app.delete("/api/documents/{document_id}")
async def delete_document(document_id: str):
    """Elimina documento da indice"""
    if not qdrant_connector:
        raise HTTPException(status_code=503, detail="Qdrant non connesso")
    
    try:
        logger.info(f"🗑️  Eliminando documento: {document_id}")
        qdrant_connector.delete_document(document_id)
        logger.info(f"✅ Documento eliminato: {document_id}")
        return {"message": f"Documento {document_id} eliminato"}
    except Exception as e:
        logger.error(f"Errore eliminazione: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))


# ============================================================================
# RAG QUERY
# ============================================================================

@app.post("/api/query", response_model=QueryResponse)
async def query_rag(request: QueryRequest, user_id: str = "default"):
    """
    Query principale - RAG Pipeline completo CON MEMORIA CONVERSAZIONALE
    
    Processa:
    1. Retrieval storia conversazione per l'utente
    2. Embedding della query
    3. Retrieval da Qdrant
    4. LLM generation con contesto storico
    5. Salva risposta in memoria
    6. Ritorna answer + sources
    """
    if not rag_pipeline:
        raise HTTPException(status_code=503, detail="RAG Pipeline non inizializzato")
    
    try:
        start_time = datetime.now()
        
        # Inizializza conversazione per questo utente se non esiste
        if user_id not in user_conversations:
            user_conversations[user_id] = []
        
        conversation_history = user_conversations[user_id]
        
        logger.info("=" * 80)
        logger.info(f"❓ QUERY (user: {user_id}): '{request.query}'")
        logger.info(f"   top_k: {request.top_k}")
        logger.info(f"   temperature: {request.temperature}")
        logger.info(f"   History length: {len(conversation_history)} scambi")
        logger.info("=" * 80)
        
        # Passa la storia al pipeline
        answer, sources = rag_pipeline.query(
            query=request.query,
            top_k=request.top_k,
            temperature=request.temperature,
            history=conversation_history  # ← MEMORIA CONVERSAZIONALE
        )
        
        # Salva il nuovo scambio nella memoria
        conversation_history.append({
            "user": request.query,
            "assistant": answer
        })
        
        # Limita a ultimi 20 scambi per non consumare memoria
        if len(conversation_history) > 20:
            user_conversations[user_id] = conversation_history[-20:]
        
        processing_time = (datetime.now() - start_time).total_seconds()
        
        logger.info("=" * 80)
        logger.info(f"✅ QUERY COMPLETATA in {processing_time:.2f}s")
        logger.info(f"   Answer length: {len(answer)} chars")
        logger.info(f"   Sources: {len(sources)}")
        for src in sources:
            logger.info(f"     - {src['filename']} (relevance: {src['similarity_score']:.2%})")
        logger.info(f"   Conversazione salvata ({len(user_conversations[user_id])} scambi)")
        logger.info("=" * 80)
        
        return QueryResponse(
            answer=answer,
            sources=[SourceInfo(**src) for src in sources],
            processing_time=processing_time,
            num_sources=len(sources)
        )
        
    except Exception as e:
        logger.error("=" * 80)
        logger.error(f"❌ ERRORE QUERY: {str(e)}")
        logger.error(traceback.format_exc())
        logger.error("=" * 80)
        raise HTTPException(status_code=500, detail=str(e))


# ============================================================================
# ADMIN ENDPOINTS
# ============================================================================

@app.post("/api/admin/reindex-all")
async def reindex_all(background_tasks: BackgroundTasks):
    """Reindicizza tutti i documenti"""
    if not rag_pipeline:
        raise HTTPException(status_code=503, detail="RAG Pipeline non inizializzato")
    
    logger.info("🔄 Avvio reindexing di tutti i documenti...")
    background_tasks.add_task(rag_pipeline.reindex_all_documents)
    return {"message": "Reindexing in corso..."}


@app.get("/api/admin/stats")
async def get_stats():
    """Statistiche del sistema"""
    if not qdrant_connector:
        raise HTTPException(status_code=503, detail="Qdrant non connesso")
    
    try:
        return qdrant_connector.get_stats()
    except Exception as e:
        logger.error(f"Errore statistiche: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))


# ============================================================================
# ROOT
# ============================================================================

@app.get("/")
async def root():
    """Endpoint root"""
    return {
        "message": "RAG Enterprise Backend v1.0.0",
        "docs": "/docs",
        "health": "/health",
        "info": "/info"
    }


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        app,
        host="0.0.0.0",
        port=8000,
        log_level="info"
    )