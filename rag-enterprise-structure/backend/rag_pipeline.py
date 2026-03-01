"""
RAG Pipeline - LangChain + Qdrant Integration
Orchestrates: Retrieval + LLM Generation with Source Attribution
"""

import json
import logging
import time
from typing import List, Tuple, Dict
import requests as _requests
from langchain.chains import RetrievalQA
from langchain.prompts import PromptTemplate
from langchain.text_splitter import RecursiveCharacterTextSplitter
from langchain_community.embeddings import HuggingFaceEmbeddings

logger = logging.getLogger(__name__)


def _format_bytes(n: int) -> str:
    """Format bytes into human-readable string."""
    for unit in ("B", "KB", "MB", "GB"):
        if abs(n) < 1024:
            return f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def _format_eta(seconds: float) -> str:
    """Format seconds into human-readable ETA."""
    if seconds < 60:
        return f"{int(seconds)}s"
    minutes = int(seconds // 60)
    secs = int(seconds % 60)
    if minutes < 60:
        return f"{minutes}m {secs}s"
    hours = int(minutes // 60)
    mins = minutes % 60
    return f"{hours}h {mins}m"


def wait_for_ollama(base_url: str, timeout: int = 300):
    """
    Wait for Ollama server to be ready.

    Args:
        base_url: Ollama base URL (e.g. http://ollama:11434)
        timeout: Maximum seconds to wait
    """
    logger.info(f"⏳ Waiting for Ollama at {base_url} ...")
    start = time.time()
    while time.time() - start < timeout:
        try:
            resp = _requests.get(f"{base_url}/api/tags", timeout=5)
            if resp.status_code == 200:
                logger.info(f"✅ Ollama is ready ({time.time() - start:.0f}s)")
                return
        except _requests.ConnectionError:
            pass
        except Exception as exc:
            logger.debug(f"Ollama not ready yet: {exc}")
        time.sleep(3)
    raise RuntimeError(
        f"Ollama not reachable at {base_url} after {timeout}s. "
        "Make sure the Ollama container is running."
    )


def ensure_model(base_url: str, model: str):
    """
    Check if a model is available in Ollama; if not, pull it with progress.

    Args:
        base_url: Ollama base URL
        model: Model name (e.g. qwen3:14b-q4_K_M)
    """
    base_url = base_url.rstrip("/")

    # Check existing models
    resp = _requests.get(f"{base_url}/api/tags", timeout=10)
    resp.raise_for_status()
    available = [m["name"] for m in resp.json().get("models", [])]

    # Ollama sometimes stores names with :latest suffix
    if model in available or f"{model}:latest" in available:
        logger.info(f"✅ Model '{model}' already available in Ollama")
        return

    # Model not found — pull it
    logger.info("=" * 70)
    logger.info(f"⬇️  Model '{model}' not found in Ollama — downloading now")
    logger.info(f"   This may take several minutes depending on your connection speed")
    logger.info("=" * 70)

    pull_resp = _requests.post(
        f"{base_url}/api/pull",
        json={"name": model, "stream": True},
        stream=True,
        timeout=3600,  # 1 hour timeout for large models
    )
    pull_resp.raise_for_status()

    start_time = time.time()
    last_log_time = 0.0
    last_status = ""

    for line in pull_resp.iter_lines():
        if not line:
            continue
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            continue

        status = data.get("status", "")
        total = data.get("total", 0)
        completed = data.get("completed", 0)

        # Show download progress every 5 seconds to avoid log spam
        now = time.time()
        if total and completed:
            pct = (completed / total) * 100
            elapsed = now - start_time
            speed = completed / elapsed if elapsed > 0 else 0
            remaining = total - completed
            eta = remaining / speed if speed > 0 else 0

            if now - last_log_time >= 5 or pct >= 100:
                logger.info(
                    f"   ⬇️  {status}: {pct:.1f}% "
                    f"({_format_bytes(completed)}/{_format_bytes(total)}) "
                    f"- Speed: {_format_bytes(speed)}/s "
                    f"- ETA: {_format_eta(eta)}"
                )
                last_log_time = now
        elif status and status != last_status:
            logger.info(f"   📦 {status}")
            last_status = status

        # Check for errors
        if "error" in data:
            raise RuntimeError(f"Ollama pull failed: {data['error']}")

    elapsed_total = time.time() - start_time
    logger.info("=" * 70)
    logger.info(
        f"✅ Model '{model}' downloaded successfully "
        f"(took {_format_eta(elapsed_total)})"
    )
    logger.info("=" * 70)


class OllamaChatDirect:
    """Direct Ollama /api/chat client with think=false support."""

    def __init__(self, model: str, base_url: str, temperature: float = 0.0,
                 timeout: int = 120):
        self.model = model
        self.base_url = base_url.rstrip("/")
        self.temperature = temperature
        self.timeout = timeout
        self._session = _requests.Session()

    def __call__(self, prompt: str) -> str:
        return self.invoke(prompt)

    def invoke(self, prompt: str) -> str:
        resp = self._session.post(
            f"{self.base_url}/api/chat",
            json={
                "model": self.model,
                "messages": [{"role": "user", "content": prompt}],
                "stream": False,
                "think": False,
                "options": {"temperature": self.temperature},
            },
            timeout=self.timeout,
        )
        resp.raise_for_status()
        return resp.json()["message"]["content"]


class RAGPipeline:
    """
    Main RAG Pipeline with Source Attribution
    - Manages retrieval from Qdrant
    - Generates responses with LLM
    - Returns sources with relevance scoring
    - Orchestrates everything with LangChain
    """
    
    def __init__(
        self,
        qdrant_connector,
        embeddings_service,
        llm_model: str = "mistral",
        ollama_base_url: str = "http://ollama:11434",
        chunk_size: int = 2000,
        chunk_overlap: int = 400,
        relevance_threshold: float = 0.30  # Lowered for better recall
    ):
        self.qdrant_connector = qdrant_connector
        self.embeddings_service = embeddings_service
        self.llm_model = llm_model
        self.relevance_threshold = relevance_threshold

        # Text splitter per chunking
        self.text_splitter = RecursiveCharacterTextSplitter(
            chunk_size=chunk_size,
            chunk_overlap=chunk_overlap,
            separators=["\n\n", "\n", ".", " ", ""]
        )

        # LLM (via Ollama) — Direct API call with think=False for Qwen3
        # Temperature 0.0 = completely deterministic to ensure consistent responses
        self.llm = OllamaChatDirect(
            model=self.llm_model,  # Use the model passed from docker-compose.yml
            base_url=ollama_base_url,
            temperature=0.0
        )
        
        # Prompt template
        self.qa_prompt = PromptTemplate(
            template=self._get_prompt_template(),
            input_variables=["context", "question"]
        )
        
        logger.info(f"✅ RAG Pipeline initialized (LLM: {llm_model}, threshold: {relevance_threshold})")
    
    
    def _get_prompt_template(self) -> str:
        """Optimized template for better extraction and accuracy"""
        return """You are a precise research assistant. Your task is to find and extract specific information from the provided documents.

INSTRUCTIONS:
1. Read ALL document chunks carefully - information may be spread across multiple chunks
2. Extract and combine relevant information from different chunks when needed
3. Quote specific names, dates, numbers, and facts exactly as they appear
4. If you find partial information, provide what you found and note what's missing
5. Only say "I don't have this information" if NONE of the chunks contain relevant data

{history_section}

DOCUMENTS:
{context}

QUESTION: {question}

ANSWER (be specific, quote facts from documents):"""
    
    
    def _format_history(self, history: List[Dict] = None) -> str:
        """
        Format conversational history - ONLY QUESTIONS

        Anti-hallucination fix: Include only user questions,
        NOT assistant responses (which could be wrong
        and create hallucination loops)
        """
        if not history or len(history) == 0:
            return ""

        history_text = "USER'S PREVIOUS QUESTIONS (for context):\n"
        for i, msg in enumerate(history[-5:], 1):  # Last 5 exchanges for better context
            user_msg = msg.get("user", "")
            if user_msg:  # Only if there's actually a question
                history_text += f"{i}. {user_msg}\n"

        return history_text + "\n"
    
    
    def chunk_text(
        self,
        text: str,
        chunk_size: int = 2000,
        overlap: int = 400
    ) -> List[str]:
        """
        Split text into chunks

        Args:
            text: Text to split
            chunk_size: Maximum chunk size
            overlap: Overlap between chunks

        Returns:
            List of chunks
        """
        splitter = RecursiveCharacterTextSplitter(
            chunk_size=chunk_size,
            chunk_overlap=overlap
        )
        chunks = splitter.split_text(text)
        logger.info(f"📊 Text split into {len(chunks)} chunks (size={chunk_size}, overlap={overlap})")
        return chunks
    
    
    def index_chunks(
        self,
        chunks: List[str],
        document_id: str,
        filename: str,
        document_type: str = "GENERIC_DOCUMENT",
        structured_fields: dict = None
    ):
        if structured_fields is None:
            structured_fields = {}

        """
        Index chunks on Qdrant
        1. Generate embeddings for each chunk
        2. Save on Qdrant with complete metadata

        Args:
            chunks: List of text chunks
            document_id: Unique document ID
            filename: Original file name
        """
        try:
            if not chunks:
                logger.warning(f"⚠️  No chunks to index for {filename}")
                return

            logger.info(f"📇 Indexing {len(chunks)} chunks for '{filename}'")
            
            # 1. Generate embeddings
            logger.debug(f"  1/2 Generating embeddings...")
            embeddings = self.embeddings_service.embed_texts(chunks)

            if not embeddings:
                logger.error(f"❌ Embedding service returned empty list!")
                return

            logger.info(f"      ✅ {len(embeddings)} embeddings generated")

            # 2. Prepare metadata
            metadatas = [
                {
                    "document_id": document_id,
                    "filename": filename,
                    "chunk_index": i,
                    "text": chunk,
                    "chunk_size": len(chunk),
                    "document_type": document_type,
                    "structured_fields": str(structured_fields),
                }
                for i, chunk in enumerate(chunks)
            ]

            logger.debug(f"  2/2 Saving on Qdrant...")

            # 3. Save on Qdrant
            self.qdrant_connector.insert_vectors(
                vectors=embeddings,
                metadatas=metadatas
            )

            logger.info(f"✅ Indexing completed for '{filename}' ({len(chunks)} chunks)")

        except Exception as e:
            logger.error(f"❌ Indexing error: {str(e)}")
            raise
    
    
    def query(
        self,
        query: str,
        top_k: int = 15,
        temperature: float = 0.7,
        history: List[Dict] = None
    ) -> Tuple[str, List[Dict]]:
        """
        Execute complete RAG query
        1. Retrieval from Qdrant with relevance scoring
        2. LLM generation
        3. Return answer + filtered sources

        Args:
            query: Query text
            top_k: Maximum number of documents to retrieve
            temperature: LLM temperature (0.0-1.0)

        Returns:
            Tuple (answer_text, list_of_sources)
        """
        try:
            logger.info(f"❓ RAG Query: '{query}' (top_k={top_k}, threshold={self.relevance_threshold})")
            
            # 1. Retrieval from Qdrant
            logger.debug("  1/3 Retrieval from Qdrant...")
            query_embedding = self.embeddings_service.embed_text(query)

            if query_embedding is None:
                logger.error("❌ Query embedding is None!")
                return "Error during query processing", []

            retrieved_docs = self.qdrant_connector.search(
                query_vector=query_embedding,
                top_k=top_k,
                score_threshold=self.relevance_threshold  # ✅ FIX: Filter upstream in Qdrant
            )

            logger.info(f"      ✅ Retrieved {len(retrieved_docs)} documents (already filtered by Qdrant with threshold={self.relevance_threshold})")

            # Detailed log of retrieved documents
            if retrieved_docs:
                logger.info("      📊 Similarity scores:")
                for i, doc in enumerate(retrieved_docs, 1):
                    filename = doc["metadata"].get("filename", "unknown")
                    similarity = doc.get("similarity", 0)
                    logger.info(f"         {i}. {filename}: {similarity:.3f} ({similarity:.1%})")
            
            if not retrieved_docs:
                logger.warning("⚠️  Qdrant returned no results above threshold!")
                logger.warning(f"⚠️  Possible causes: threshold too high ({self.relevance_threshold}) or non-relevant documents")
                return "I haven't found relevant documents to answer this question.", []

            # 🎯 Keep more documents for complex queries - less aggressive filtering
            # Only filter if there's a VERY clear winner with huge gap
            if len(retrieved_docs) > 1:
                first_score = retrieved_docs[0].get("similarity", 0)
                second_score = retrieved_docs[1].get("similarity", 0)
                gap = first_score - second_score

                # Only filter if gap is very large (>0.15) AND top score is high (>0.65)
                # This preserves more context for complex questions
                if first_score >= 0.65 and gap > 0.15:
                    logger.info(f"      🎯 Gap filtering activated: top_score={first_score:.3f}, gap={gap:.3f}")
                    relevant_docs = [doc for doc in retrieved_docs if doc.get("similarity", 0) >= 0.40]
                    logger.info(f"      ✅ Gap filtering: {len(retrieved_docs)} → {len(relevant_docs)} documents (filtered < 0.40)")

                    # Safety check: keep at least top 3 documents
                    if len(relevant_docs) < 3:
                        logger.warning("⚠️  Gap filtering too aggressive, keeping top 3")
                        relevant_docs = retrieved_docs[:3]
                else:
                    relevant_docs = retrieved_docs
                    logger.info(f"      ✅ Keeping all {len(relevant_docs)} documents for comprehensive context")
            else:
                relevant_docs = retrieved_docs
                logger.info(f"      ✅ {len(relevant_docs)} relevant document")
            
            # 3. Build context from search
            logger.debug("  2/3 Creating context...")
            context_parts = []
            for i, doc in enumerate(relevant_docs, 1):
                text = doc["metadata"].get("text", "")
                filename = doc["metadata"].get("filename", "unknown")
                similarity = doc.get("similarity", 0)

                context_parts.append(
                    f"[{i}] ({filename} - relevance: {similarity:.2%})\n{text}"
                )

            context = "\n\n---\n\n".join(context_parts)
            logger.debug(f"      Context length: {len(context)} chars")

            # 4. LLM Generation
            logger.debug("  3/3 LLM Generation...")

            # Format conversational history
            history_section = self._format_history(history)

            prompt = self.qa_prompt.format(
                history_section=history_section,
                context=context,
                question=query
            )

            logger.debug(f"      Prompt length: {len(prompt)} chars")

            # Call LLM
            answer = self.llm(prompt)
            logger.info(f"      ✅ Response generated ({len(answer)} characters)")

            # 5. Format sources - DEDUPLICATED per document
            logger.debug("  Formatting sources...")
            sources_dict = {}  # Use dict for deduplication

            for doc in relevant_docs:
                doc_id = doc["metadata"].get("document_id", "unknown")
                filename = doc["metadata"].get("filename", "unknown")
                similarity = doc.get("similarity", 0)

                # Use the document with highest similarity
                if doc_id not in sources_dict or similarity > sources_dict[doc_id]["similarity_score"]:
                    sources_dict[doc_id] = {
                        "filename": filename,
                        "document_id": doc_id,
                        "similarity_score": round(similarity, 3),
                        "chunk_index": doc["metadata"].get("chunk_index", 0),
                        "text": doc["metadata"].get("text", "")
                    }

            sources = list(sources_dict.values())
            # Sort by descending similarity
            sources.sort(key=lambda x: x["similarity_score"], reverse=True)

            logger.info(f"✅ Query completed - {len(sources)} unique sources returned")

            return answer, sources
            
        except Exception as e:
            logger.error(f"❌ Query error: {str(e)}", exc_info=True)
            raise


    def reindex_all_documents(self):
        """Reindex all documents (if needed)"""
        try:
            logger.info("📄 Reindexing all documents...")
            # Implementation depends on how you save the originals
            # This is a skeleton for future implementations
            logger.info("✅ Reindexing completed")
        except Exception as e:
            logger.error(f"❌ Reindexing error: {str(e)}")
            raise