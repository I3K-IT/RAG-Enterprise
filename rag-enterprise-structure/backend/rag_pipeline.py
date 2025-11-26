"""
RAG Pipeline - LangChain + Qdrant Integration
Orchestrates: Retrieval + LLM Generation with Source Attribution
"""

import logging
from typing import List, Tuple, Dict
from langchain.chains import RetrievalQA
from langchain.prompts import PromptTemplate
from langchain.text_splitter import RecursiveCharacterTextSplitter
from langchain_community.llms import Ollama
from langchain_community.embeddings import HuggingFaceEmbeddings

logger = logging.getLogger(__name__)


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
        chunk_size: int = 1000,
        chunk_overlap: int = 100,
        relevance_threshold: float = 0.5  # Relevance filter
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
        
        # LLM (via Ollama)
        # Temperature 0.0 = completely deterministic to ensure consistent responses
        self.llm = Ollama(
            model=self.llm_model,  # Use the model passed from docker-compose.yml
            base_url="http://ollama:11434",
            temperature=0.0
        )
        
        # Prompt template
        self.qa_prompt = PromptTemplate(
            template=self._get_prompt_template(),
            input_variables=["context", "question"]
        )
        
        logger.info(f"✅ RAG Pipeline initialized (LLM: {llm_model}, threshold: {relevance_threshold})")
    
    
    def _get_prompt_template(self) -> str:
        """Simplified template to reduce hallucinations"""
        return """You are an assistant that answers ONLY using information present in the provided documents.

CRITICAL RULES:
1. Use ONLY information present in the CONTEXT below
2. If the information is NOT in the context → answer "I don't have this information in the documents"
3. Do NOT invent, do NOT assume, do NOT generate data
4. If there are multiple documents, use the one with the highest relevance
5. The Italian TAX CODE has exactly 16 characters (e.g., MRCFNC69E20E329H)
6. Verify that the name in the document matches the name in the question

{history_section}

CONTEXT:
{context}

QUESTION: {question}

ANSWER (use ONLY info from context):"""
    
    
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
        for i, msg in enumerate(history[-3:], 1):  # Last 3 exchanges (reduced from 5)
            user_msg = msg.get("user", "")
            if user_msg:  # Only if there's actually a question
                history_text += f"{i}. {user_msg}\n"

        return history_text + "\n"
    
    
    def chunk_text(
        self,
        text: str,
        chunk_size: int = 1000,
        overlap: int = 100
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
        top_k: int = 5,
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

            # Log dettagliato dei documenti recuperati
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

            # 🎯 Intelligent gap-based filtering to reduce "noise dilution"
            if len(retrieved_docs) > 1:
                first_score = retrieved_docs[0].get("similarity", 0)
                second_score = retrieved_docs[1].get("similarity", 0)
                gap = first_score - second_score

                # If there's a CLEARLY more relevant document (high score + significant gap)
                # filter out documents with medium-low scores to avoid confusion in the LLM
                # Lowered thresholds to be more aggressive: 0.60→0.50, 0.12→0.08
                if first_score >= 0.50 and gap > 0.08:
                    logger.info(f"      🎯 Gap filtering activated: top_score={first_score:.3f}, gap={gap:.3f}")
                    relevant_docs = [doc for doc in retrieved_docs if doc.get("similarity", 0) >= 0.45]
                    logger.info(f"      ✅ Gap filtering: {len(retrieved_docs)} → {len(relevant_docs)} documents (filtered < 0.45)")

                    # Safety check: keep at least the first document
                    if not relevant_docs:
                        logger.warning("⚠️  Gap filtering removed ALL documents, keeping the first one")
                        relevant_docs = [retrieved_docs[0]]
                else:
                    relevant_docs = retrieved_docs
                    logger.info(f"      ✅ {len(relevant_docs)} relevant documents (gap={gap:.3f}, below threshold 0.08)")
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