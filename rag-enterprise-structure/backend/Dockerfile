FROM nvidia/cuda:12.9.0-runtime-ubuntu22.04

ENV HF_HOME=/root/.cache/huggingface
ENV TRANSFORMERS_CACHE=/root/.cache/huggingface
ENV SENTENCE_TRANSFORMERS_HOME=/root/.cache/huggingface

WORKDIR /app

# Install Python and system dependencies
RUN apt-get update && apt-get install -y \
    python3.10 \
    python3-pip \
    git \
    libsm6 \
    libxext6 \
    libxrender-dev \
    libgl1-mesa-glx \
    libglib2.0-0 \
    libgomp1 \
	tesseract-ocr \
    libtesseract-dev \
    poppler-utils \
    && rm -rf /var/lib/apt/lists/*

# Install Tika to support all formats
RUN apt-get update -qq && apt-get install -y -qq default-jre wget curl
RUN mkdir -p /opt && wget -q https://archive.apache.org/dist/tika/3.2.3/tika-server-standard-3.2.3.jar -O /opt/tika-server.jar

# Install PyTorch with CUDA 12.8 from official index
RUN pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu128 --no-cache-dir

# Upgrade pip
RUN pip install --upgrade pip setuptools wheel

# Copy requirements
COPY backend/requirements.txt .

# Install Python dependencies
RUN pip install --no-cache-dir -r requirements.txt

# Copy backend code
COPY backend/app.py .
COPY backend/rag_pipeline.py .
COPY backend/ocr_service.py .
COPY backend/embeddings_service.py .
COPY backend/qdrant_connector.py .

# Copy authentication files
COPY backend/auth.py .
COPY backend/auth_models.py .
COPY backend/database.py .
COPY backend/middleware.py .

# Create directory for uploads and database
RUN mkdir -p /app/uploads /app/data

# Expose port
EXPOSE 8000

# Environment variables
ENV MILVIUS_HOST=milvus
ENV MILVIUS_PORT=19530
ENV LLM_MODEL=mistral
ENV EMBEDDING_MODEL=all-MiniLM-L6-v2
ENV CUDA_VISIBLE_DEVICES=0

# Health check - increased timeout for large document processing
HEALTHCHECK --interval=30s --timeout=120s --start-period=60s --retries=5 \
    CMD curl -f http://localhost:8000/health || exit 1

# Start app
CMD ["python3", "-m", "uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8000"]
