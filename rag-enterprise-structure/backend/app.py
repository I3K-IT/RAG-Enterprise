"""
RAG Enterprise Backend - FastAPI Application
Manages: OCR, Embedding, RAG Pipeline, Qdrant Integration
"""

from fastapi import FastAPI, File, UploadFile, HTTPException, BackgroundTasks, Depends
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

# Authentication imports
from auth import create_user_token
from database import db, UserRole
from auth_models import (
    LoginRequest, LoginResponse, UserInfo, UserCreate, UserUpdate,
    PasswordChange, UserListResponse, MessageResponse
)
from middleware import (
    get_current_user, require_admin, require_upload_permission,
    require_delete_permission, CurrentUser
)

def detect_document_type(text: str) -> str:
    """Detects document type - with stricter checks"""
    text_upper = text.upper()

    # Order: more specific → less specific

    # 1. IDENTITY CARD (very specific)
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
    """Extracts fields from Identity Card - vertical layout"""
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
    """Extracts fields from Passport"""
    fields = {}
    
    # Numero passaporto (di solito 9 caratteri)
    passport_pattern = r'[A-Z]{2}\d{7}'
    passport_match = re.search(passport_pattern, text)
    if passport_match:
        fields['numero_passaporto'] = passport_match.group()
    
    return fields


def extract_license_fields(text: str) -> Dict[str, Optional[str]]:
    """Extracts fields from Driving License - WITH strict checks"""
    fields = {}

    # Check 1: must contain "PATENTE DI GUIDA"
    if 'PATENTE DI GUIDA' not in text.upper() and 'DRIVING LICENSE' not in text.upper():
        return fields

    # Check 2: Italian license number pattern (10 alphanumeric characters)
    # But ONLY if preceded by specific keywords
    license_pattern = r'(?:Numero|Number|N\.|Nr\.)\s*[:\s]*([A-Z0-9]{10})'
    license_match = re.search(license_pattern, text)
    if license_match:
        fields['numero_patente'] = license_match.group(1)
    
    return fields


def extract_structured_fields(text: str, doc_type: str) -> Dict[str, Optional[str]]:
    """Extracts structured fields based on document type"""

    if doc_type == 'IDENTITY_CARD':
        return extract_id_fields(text)
    elif doc_type == 'PASSPORT':
        return extract_passport_fields(text)
 #   elif doc_type == 'DRIVING_LICENSE':
 #       return extract_license_fields(text)
    else:
        return {}

# Logging setup - MORE DETAILED
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

# Create upload folder if it doesn't exist
os.makedirs(UPLOAD_DIR, exist_ok=True)

# FastAPI App
app = FastAPI(
    title="RAG Enterprise Backend",
    description="API for Distributed RAG Pipeline",
    version="1.0.0"
)

# CORS configuration
# Read ALLOWED_ORIGINS from environment variable
allowed_origins_env = os.getenv("ALLOWED_ORIGINS", "")
if allowed_origins_env:
    # Split by comma and strip whitespace
    allowed_origins = [origin.strip() for origin in allowed_origins_env.split(",")]
    logging.info(f"CORS: Restricted to specific origins: {allowed_origins}")
else:
    # Default: allow all (development mode)
    allowed_origins = ["*"]
    logging.warning("CORS: ALLOWED_ORIGINS not set - allowing all origins (*)")
    logging.warning("For production, set ALLOWED_ORIGINS in .env (e.g., https://yourdomain.com)")

app.add_middleware(
    CORSMiddleware,
    allow_origins=allowed_origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Global services
ocr_service: Optional[OCRService] = None
embeddings_service: Optional[EmbeddingsService] = None
rag_pipeline: Optional[RAGPipeline] = None
qdrant_connector: Optional[QdrantConnector] = None

# Conversational memory for users
user_conversations: dict = {}  # {user_id: [{"user": "...", "assistant": "..."}]}


# ============================================================================
# INITIALIZATION
# ============================================================================

@app.on_event("startup")
async def startup_event():
    """Initialize services at startup"""
    global ocr_service, embeddings_service, rag_pipeline, qdrant_connector

    logger.info("=" * 80)
    logger.info("🚀 STARTING RAG BACKEND")
    logger.info("=" * 80)
    logger.info(f"Configuration:")
    logger.info(f"  - QDRANT: {QDRANT_HOST}:{QDRANT_PORT}")
    logger.info(f"  - LLM: {LLM_MODEL}")
    logger.info(f"  - Embedding: {EMBEDDING_MODEL}")
    logger.info(f"  - Relevance Threshold: {RELEVANCE_THRESHOLD}")
    logger.info(f"  - Upload Dir: {UPLOAD_DIR}")
    logger.info(f"  - CUDA Devices: {CUDA_VISIBLE_DEVICES}")
    logger.info("=" * 80)
    
    try:
        # 1. Qdrant Connection
        logger.info("🔗 [1/4] Connecting to Qdrant...")
        qdrant_connector = QdrantConnector(
            host="qdrant",
            port=6333
        )
        qdrant_connector.connect()
        logger.info("✅ Qdrant connected")

        # 2. OCR Service
        logger.info("🔗 [2/4] Loading OCR Service...")
        try:
            ocr_service = OCRService()
            logger.info("✅ OCR Service ready")
        except Exception as e:
            logger.warning(f"⚠️  OCR Service failed: {e}")
            logger.warning("    → System will continue without OCR")
            ocr_service = None

        # 3. Embedding Service
        logger.info(f"🔗 [3/4] Loading Embedding Service ({EMBEDDING_MODEL})...")
        embeddings_service = EmbeddingsService(model_name=EMBEDDING_MODEL)
        logger.info("✅ Embedding Service ready")

        # 4. RAG Pipeline
        logger.info(f"🔗 [4/4] Initializing RAG Pipeline (LLM: {LLM_MODEL})...")
        rag_pipeline = RAGPipeline(
            qdrant_connector=qdrant_connector,
            embeddings_service=embeddings_service,
            llm_model=LLM_MODEL,
            relevance_threshold=RELEVANCE_THRESHOLD
        )
        logger.info("✅ RAG Pipeline ready")

        logger.info("=" * 80)
        logger.info("🎉 BACKEND FULLY INITIALIZED")
        logger.info("=" * 80)

    except Exception as e:
        logger.error("=" * 80)
        logger.error(f"❌ ERROR DURING STARTUP: {str(e)}")
        logger.error(traceback.format_exc())
        logger.error("=" * 80)
        raise


@app.on_event("shutdown")
async def shutdown_event():
    """Cleanup at shutdown"""
    logger.info("🛑 Shutting down RAG Backend...")
    if qdrant_connector:
        qdrant_connector.disconnect()
    logger.info("✅ Cleanup completed")


# ============================================================================
# PYDANTIC MODELS
# ============================================================================

class QueryRequest(BaseModel):
    query: str
    top_k: int = 5
    temperature: float = 0.0  # 0.0 = completely deterministic, eliminates variability in responses


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
    """Configuration information"""
    return {
        "backend": "RAG Enterprise v1.0.0",
        "qdrant": f"{QDRANT_HOST}:{QDRANT_PORT}",
        "llm_model": LLM_MODEL,
        "embedding_model": EMBEDDING_MODEL,
        "cuda_devices": CUDA_VISIBLE_DEVICES,
        "relevance_threshold": RELEVANCE_THRESHOLD,
    }


# ============================================================================
# AUTHENTICATION ENDPOINTS
# ============================================================================

@app.post("/api/auth/login", response_model=LoginResponse)
async def login(request: LoginRequest):
    """
    User login - returns JWT token

    Default credentials:
    - Admin: username=admin, password=admin123
    """
    user = db.authenticate_user(request.username, request.password)

    if not user:
        raise HTTPException(
            status_code=401,
            detail="Incorrect username or password"
        )

    # Create JWT token
    token = create_user_token(user)

    logger.info(f"✅ Login successful: {user['username']} (role: {user['role']})")

    return LoginResponse(
        access_token=token,
        token_type="bearer",
        user=UserInfo(
            id=user["id"],
            username=user["username"],
            email=user["email"],
            role=user["role"],
            created_at=user["created_at"],
            last_login=user.get("last_login")
        )
    )


@app.get("/api/auth/me", response_model=UserInfo)
async def get_current_user_info(current_user: CurrentUser = Depends(get_current_user)):
    """Get current user information"""
    user = db.get_user_by_id(current_user.user_id)

    return UserInfo(
        id=user["id"],
        username=user["username"],
        email=user["email"],
        role=user["role"],
        created_at=user["created_at"],
        last_login=user.get("last_login")
    )


@app.get("/api/auth/users", response_model=UserListResponse)
async def list_users(current_user: CurrentUser = Depends(require_admin)):
    """List all users (ADMIN only)"""
    users = db.list_users()

    return UserListResponse(
        users=[
            UserInfo(
                id=u["id"],
                username=u["username"],
                email=u["email"],
                role=u["role"],
                created_at=u["created_at"],
                last_login=u.get("last_login")
            )
            for u in users
        ],
        total=len(users)
    )


@app.post("/api/auth/users", response_model=UserInfo)
async def create_user(
    user_data: UserCreate,
    current_user: CurrentUser = Depends(require_admin)
):
    """Create new user (ADMIN only)"""
    user_id = db.create_user(
        username=user_data.username,
        email=user_data.email,
        password=user_data.password,
        role=user_data.role
    )

    if not user_id:
        raise HTTPException(
            status_code=400,
            detail="User creation error (username or email already exists)"
        )

    user = db.get_user_by_id(user_id)

    return UserInfo(
        id=user["id"],
        username=user["username"],
        email=user["email"],
        role=user["role"],
        created_at=user["created_at"],
        last_login=user.get("last_login")
    )


@app.put("/api/auth/users/{user_id}", response_model=MessageResponse)
async def update_user(
    user_id: int,
    user_data: UserUpdate,
    current_user: CurrentUser = Depends(require_admin)
):
    """Update user (ADMIN only)"""
    if user_data.role:
        success = db.update_user_role(user_id, user_data.role)
        if not success:
            raise HTTPException(status_code=404, detail="User not found")

    return MessageResponse(message=f"User {user_id} updated")


@app.delete("/api/auth/users/{user_id}", response_model=MessageResponse)
async def delete_user(
    user_id: int,
    current_user: CurrentUser = Depends(require_admin)
):
    """Delete user (ADMIN only)"""
    # Don't allow self-deletion
    if user_id == current_user.user_id:
        raise HTTPException(
            status_code=400,
            detail="You cannot delete your own account"
        )

    success = db.delete_user(user_id)
    if not success:
        raise HTTPException(status_code=404, detail="User not found")

    return MessageResponse(message=f"User {user_id} deleted")


@app.post("/api/auth/change-password", response_model=MessageResponse)
async def change_password(
    request: PasswordChange,
    current_user: CurrentUser = Depends(get_current_user)
):
    """Change current user's password"""
    user = db.get_user_by_id(current_user.user_id)

    # Verify old password
    if not db.verify_password(request.old_password, user["password_hash"]):
        raise HTTPException(
            status_code=400,
            detail="Current password is incorrect"
        )

    # Change password
    success = db.change_password(current_user.user_id, request.new_password)

    if not success:
        raise HTTPException(status_code=500, detail="Password change error")

    logger.info(f"✅ Password changed for user: {current_user.username}")

    return MessageResponse(message="Password changed successfully")


# ============================================================================
# DOCUMENT MANAGEMENT
# ============================================================================

# Supported formats
ALLOWED_EXTENSIONS = {
    '.pdf', '.txt', '.doc', '.docx', '.ppt', '.pptx', '.xls', '.xlsx',
    '.odt', '.rtf', '.html', '.xml', '.json', '.csv', '.md',
    '.jpg', '.jpeg', '.png', '.gif', '.bmp'
}

@app.post("/api/documents/upload")
async def upload_document(
    file: UploadFile = File(...),
    background_tasks: BackgroundTasks = None,
    current_user: CurrentUser = Depends(require_upload_permission)
):
    """
    Upload a document (any format) and process it in the background
    Supported formats: PDF, DOCX, PPTX, XLSX, ODT, RTF, HTML, XML, JSON, CSV, Images

    Requires: SUPER_USER or ADMIN role
    """

    if not ocr_service or not embeddings_service or not rag_pipeline:
        raise HTTPException(
            status_code=503,
            detail="Services not initialized. Check /health"
        )

    # Check file extension
    from pathlib import Path
    file_ext = Path(file.filename).suffix.lower()

    if file_ext not in ALLOWED_EXTENSIONS:
        raise HTTPException(
            status_code=400,
            detail=f"Format '{file_ext}' not supported. Supported: {', '.join(sorted(ALLOWED_EXTENSIONS))}"
        )
    
    try:
        # Create document_id with timestamp FIRST
        document_id = f"{datetime.now().timestamp()}_{file.filename}"
        file_path = os.path.join(UPLOAD_DIR, document_id)
        content = await file.read()

        # Save file with the document_id (with timestamp)
        with open(file_path, "wb") as f:
            f.write(content)

        logger.info(f"📄 File received: '{file.filename}' ({len(content)} bytes)")
        logger.info(f"   Document ID: {document_id}")
        logger.info(f"   File path: {file_path}")

        # Add background task
        background_tasks.add_task(
            process_document_background,
            file_path,
            document_id,
            file.filename
        )

        return JSONResponse(
            status_code=202,
            content={
                "message": "Document received, processing in progress",
                "document_id": document_id,
                "filename": file.filename,
                "size_bytes": len(content)
            }
        )

    except Exception as e:
        logger.error(f"❌ Upload error: {str(e)}")
        logger.error(traceback.format_exc())
        raise HTTPException(status_code=500, detail=str(e))


async def process_document_background(file_path: str, document_id: str, filename: str):
    """Background task to process document - DETAILED LOGGING"""
    try:
        logger.info("=" * 80)
        logger.info(f"📇 PROCESSING START: {filename}")
        logger.info(f"   Document ID: {document_id}")
        logger.info(f"   File path: {file_path}")
        logger.info("=" * 80)

        # STEP 1: OCR Extraction
        logger.info(f"  [1/3] OCR Extraction...")
        start_ocr = datetime.now()

        try:
            text = ocr_service.extract_text(file_path)

            # NEW: Detect document type and extract structured fields
            doc_type = detect_document_type(text)
            structured_fields = extract_structured_fields(text, doc_type)

            logger.info(f"Document Type: {doc_type}")
            logger.info(f"Structured Fields: {structured_fields}")
        except Exception as e:
            logger.error(f"      ❌ OCR FAILED: {str(e)}", exc_info=True)
            text = ""

        ocr_time = (datetime.now() - start_ocr).total_seconds()
        logger.info(f"        ✅ Extracted {len(text)} characters in {ocr_time:.2f}s")

        if not text or len(text.strip()) == 0:
            logger.warning(f"⚠️  WARNING: OCR returned empty text!")
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
        logger.info(f"        ✅ {len(chunks)} chunks created in {chunk_time:.2f}s")

        if not chunks:
            logger.error(f"❌ ERROR: No chunks created!")
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
        logger.info(f"        ✅ Indexed on Qdrant in {index_time:.2f}s")

        # SUMMARY
        total_time = (datetime.now() - start_ocr).total_seconds()
        logger.info("=" * 80)
        logger.info(f"✅ PROCESSING COMPLETED: {filename}")
        logger.info(f"   Total time: {total_time:.2f}s")
        logger.info(f"   - OCR: {ocr_time:.2f}s")
        logger.info(f"   - Chunking: {chunk_time:.2f}s")
        logger.info(f"   - Indexing: {index_time:.2f}s")
        logger.info(f"   Chunks: {len(chunks)}")
        logger.info(f"   Characters: {len(text)}")
        logger.info("=" * 80)

    except Exception as e:
        logger.error("=" * 80)
        logger.error(f"❌ CRITICAL PROCESSING ERROR {filename}: {str(e)}")
        logger.error(traceback.format_exc())
        logger.error("=" * 80)


@app.get("/api/documents", response_model=DocumentsListResponse)
async def list_documents():
    """List indexed documents"""
    if not qdrant_connector:
        raise HTTPException(status_code=503, detail="Qdrant not connected")

    try:
        docs = qdrant_connector.get_indexed_documents()
        return DocumentsListResponse(
            documents=docs,
            total=len(docs)
        )
    except Exception as e:
        logger.error(f"Error listing documents: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/api/documents/{document_id}/download")
async def download_document(document_id: str):
    """Download the original uploaded document"""
    from fastapi.responses import FileResponse
    import glob

    try:
        # Search for the file in the upload folder
        # The document_id has format: timestamp_filename.ext
        search_pattern = os.path.join(UPLOAD_DIR, f"{document_id}")

        # Check if the file exists exactly
        if os.path.exists(search_pattern):
            file_path = search_pattern
        else:
            # Otherwise search with wildcard (in case the path is slightly different)
            files = glob.glob(search_pattern)
            if not files:
                raise HTTPException(status_code=404, detail=f"Document not found: {document_id}")
            file_path = files[0]

        logger.info(f"📥 Download document: {document_id}")
        logger.info(f"   Path: {file_path}")

        if not os.path.exists(file_path):
            raise HTTPException(status_code=404, detail="File not found")

        # Extract the original file name (without timestamp)
        # Format: 1762533561.231156_TU-81-08-Ed.-Gennaio-2025-1.pdf
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
        logger.error(f"❌ Download error: {str(e)}")
        logger.error(traceback.format_exc())
        raise HTTPException(status_code=500, detail=str(e))


@app.delete("/api/documents/{document_id}")
async def delete_document(
    document_id: str,
    current_user: CurrentUser = Depends(require_delete_permission)
):
    """
    Delete document from index

    Requires: SUPER_USER or ADMIN role
    """
    if not qdrant_connector:
        raise HTTPException(status_code=503, detail="Qdrant not connected")

    try:
        logger.info(f"🗑️  Deleting document: {document_id}")
        qdrant_connector.delete_document(document_id)
        logger.info(f"✅ Document deleted: {document_id}")
        return {"message": f"Document {document_id} deleted"}
    except Exception as e:
        logger.error(f"Deletion error: {str(e)}")
        raise HTTPException(status_code=500, detail=str(e))


# ============================================================================
# RAG QUERY
# ============================================================================

@app.post("/api/query", response_model=QueryResponse)
async def query_rag(
    request: QueryRequest,
    current_user: CurrentUser = Depends(get_current_user)
):
    """
    Main query - Complete RAG Pipeline WITH CONVERSATIONAL MEMORY

    Requires: Authentication (all roles can make queries)

    Processes:
    1. Retrieve conversation history for the user
    2. Query embedding
    3. Retrieval from Qdrant
    4. LLM generation with historical context
    5. Save response in memory
    6. Return answer + sources
    """

    # Use real user ID instead of "default"
    user_id = str(current_user.user_id)
    if not rag_pipeline:
        raise HTTPException(status_code=503, detail="RAG Pipeline not initialized")
    
    try:
        start_time = datetime.now()

        # Initialize conversation for this user if it doesn't exist
        if user_id not in user_conversations:
            user_conversations[user_id] = []

        conversation_history = user_conversations[user_id]

        logger.info("=" * 80)
        logger.info(f"❓ QUERY (user: {user_id}): '{request.query}'")
        logger.info(f"   top_k: {request.top_k}")
        logger.info(f"   temperature: {request.temperature}")
        logger.info(f"   History length: {len(conversation_history)} exchanges")
        logger.info("=" * 80)

        # Pass history to the pipeline
        answer, sources = rag_pipeline.query(
            query=request.query,
            top_k=request.top_k,
            temperature=request.temperature,
            history=conversation_history  # ← CONVERSATIONAL MEMORY
        )

        # Save the new exchange in memory
        conversation_history.append({
            "user": request.query,
            "assistant": answer
        })

        # Limit to last 20 exchanges to not consume too much memory
        if len(conversation_history) > 20:
            user_conversations[user_id] = conversation_history[-20:]

        processing_time = (datetime.now() - start_time).total_seconds()

        logger.info("=" * 80)
        logger.info(f"✅ QUERY COMPLETED in {processing_time:.2f}s")
        logger.info(f"   Answer length: {len(answer)} chars")
        logger.info(f"   Sources: {len(sources)}")
        for src in sources:
            logger.info(f"     - {src['filename']} (relevance: {src['similarity_score']:.2%})")
        logger.info(f"   Conversation saved ({len(user_conversations[user_id])} exchanges)")
        logger.info("=" * 80)
        
        return QueryResponse(
            answer=answer,
            sources=[SourceInfo(**src) for src in sources],
            processing_time=processing_time,
            num_sources=len(sources)
        )
        
    except Exception as e:
        logger.error("=" * 80)
        logger.error(f"❌ QUERY ERROR: {str(e)}")
        logger.error(traceback.format_exc())
        logger.error("=" * 80)
        raise HTTPException(status_code=500, detail=str(e))


# ============================================================================
# ADMIN ENDPOINTS
# ============================================================================

@app.post("/api/admin/reindex-all")
async def reindex_all(background_tasks: BackgroundTasks):
    """Reindex all documents"""
    if not rag_pipeline:
        raise HTTPException(status_code=503, detail="RAG Pipeline not initialized")

    logger.info("🔄 Starting reindexing of all documents...")
    background_tasks.add_task(rag_pipeline.reindex_all_documents)
    return {"message": "Reindexing in progress..."}


@app.delete("/api/admin/memory/{user_id}")
async def clear_user_memory(user_id: str):
    """Clear conversational memory for a specific user"""
    if user_id in user_conversations:
        num_exchanges = len(user_conversations[user_id])
        del user_conversations[user_id]
        logger.info(f"🧹 Memory cleared for user '{user_id}' ({num_exchanges} exchanges removed)")
        return {
            "message": f"Memory cleared for user '{user_id}'",
            "exchanges_removed": num_exchanges
        }
    else:
        return {
            "message": f"No memory found for user '{user_id}'",
            "exchanges_removed": 0
        }


@app.delete("/api/admin/memory")
async def clear_all_memory():
    """Clear ALL conversational memory for all users"""
    total_users = len(user_conversations)
    total_exchanges = sum(len(conv) for conv in user_conversations.values())
    user_conversations.clear()
    logger.info(f"🧹 Global memory cleared: {total_users} users, {total_exchanges} total exchanges")
    return {
        "message": "Global memory cleared",
        "users_removed": total_users,
        "exchanges_removed": total_exchanges
    }


@app.get("/api/admin/memory")
async def get_memory_stats():
    """Conversational memory statistics"""
    stats = {
        "total_users": len(user_conversations),
        "users": {}
    }
    for user_id, history in user_conversations.items():
        stats["users"][user_id] = {
            "exchanges": len(history),
            "last_questions": [msg["user"] for msg in history[-3:]]
        }
    return stats


@app.get("/api/admin/stats")
async def get_stats():
    """System statistics"""
    if not qdrant_connector:
        raise HTTPException(status_code=503, detail="Qdrant not connected")

    try:
        return qdrant_connector.get_stats()
    except Exception as e:
        logger.error(f"Statistics error: {str(e)}")
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