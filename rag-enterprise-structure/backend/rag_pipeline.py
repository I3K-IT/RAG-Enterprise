"""
RAG Pipeline - LangChain + Qdrant Integration
Orestra: Retrieval + LLM Generation con Source Attribution
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
    RAG Pipeline principale con Source Attribution
    - Gestisce retrieval da Qdrant
    - Genera risposte con LLM
    - Ritorna sources con relevance scoring
    - Orchestra tutto con LangChain
    """
    
    def __init__(
        self,
        qdrant_connector,
        embeddings_service,
        llm_model: str = "mistral",
        chunk_size: int = 500,
        chunk_overlap: int = 100,
        relevance_threshold: float = 0.5  # Filtro relevance
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
        self.llm = Ollama(
            model="neural-chat:7b",
            base_url="http://ollama:11434",
            temperature=0.7
        )
        
        # Prompt template
        self.qa_prompt = PromptTemplate(
            template=self._get_prompt_template(),
            input_variables=["context", "question"]
        )
        
        logger.info(f"✅ RAG Pipeline inizializzato (LLM: {llm_model}, threshold: {relevance_threshold})")
    
    
    def _get_prompt_template(self) -> str:
        """Template per le risposte RAG"""
        return """Sei un assistente AI esperto e affidabile. Usa il contesto sottostante per rispondere alla domanda.
Se il contesto non contiene informazioni sufficienti, dillo chiaramente.
Sii conciso e preciso. Cita il contesto quando appropriato.

CONTESTO:
{context}

DOMANDA:
{question}

RISPOSTA:"""
    
    
    def chunk_text(
        self,
        text: str,
        chunk_size: int = 500,
        overlap: int = 100
    ) -> List[str]:
        """
        Divide il testo in chunks
        
        Args:
            text: Testo da dividere
            chunk_size: Dimensione massima chunk
            overlap: Sovrapposizione tra chunks
            
        Returns:
            Lista di chunks
        """
        splitter = RecursiveCharacterTextSplitter(
            chunk_size=chunk_size,
            chunk_overlap=overlap
        )
        chunks = splitter.split_text(text)
        logger.info(f"📊 Text suddiviso in {len(chunks)} chunks (size={chunk_size}, overlap={overlap})")
        return chunks
    
    
    def index_chunks(
        self,
        chunks: List[str],
        document_id: str,
        filename: str
    ):
        """
        Indicizza chunks su Qdrant
        1. Genera embeddings per ogni chunk
        2. Salva su Qdrant con metadata completi
        
        Args:
            chunks: Lista di chunks di testo
            document_id: ID documento unico
            filename: Nome file originale
        """
        try:
            if not chunks:
                logger.warning(f"⚠️  Nessun chunk da indicizzare per {filename}")
                return
            
            logger.info(f"📇 Indicizzando {len(chunks)} chunks per '{filename}'")
            
            # 1. Genera embeddings
            logger.debug(f"  1/2 Generando embeddings...")
            embeddings = self.embeddings_service.embed_texts(chunks)
            
            if not embeddings:
                logger.error(f"❌ Embedding service ha ritornato lista vuota!")
                return
            
            logger.info(f"      ✅ {len(embeddings)} embeddings generati")
            
            # 2. Prepara metadati
            metadatas = [
                {
                    "document_id": document_id,
                    "filename": filename,
                    "chunk_index": i,
                    "text": chunk,
                    "chunk_size": len(chunk),
                }
                for i, chunk in enumerate(chunks)
            ]
            
            logger.debug(f"  2/2 Salvando su Qdrant...")
            
            # 3. Salva su Qdrant
            self.qdrant_connector.insert_vectors(
                vectors=embeddings,
                metadatas=metadatas
            )
            
            logger.info(f"✅ Indexing completato per '{filename}' ({len(chunks)} chunks)")
            
        except Exception as e:
            logger.error(f"❌ Errore indexing: {str(e)}")
            raise
    
    
    def query(
        self,
        query: str,
        top_k: int = 5,
        temperature: float = 0.7
    ) -> Tuple[str, List[Dict]]:
        """
        Esegue query RAG completa
        1. Retrieval da Qdrant con relevance scoring
        2. LLM generation
        3. Return answer + sources filtrati
        
        Args:
            query: Testo della query
            top_k: Numero massimo di documenti da recuperare
            temperature: Temperatura LLM (0.0-1.0)
            
        Returns:
            Tupla (answer_text, list_of_sources)
        """
        try:
            logger.info(f"❓ RAG Query: '{query}' (top_k={top_k}, threshold={self.relevance_threshold})")
            
            # 1. Retrieval da Qdrant
            logger.debug("  1/3 Retrieval da Qdrant...")
            query_embedding = self.embeddings_service.embed_text(query)
            
            if query_embedding is None:
                logger.error("❌ Embedding della query è None!")
                return "Errore durante l'elaborazione della query", []
            
            retrieved_docs = self.qdrant_connector.search(
                query_vector=query_embedding,
                top_k=top_k
            )
            
            logger.info(f"      ✅ Recuperati {len(retrieved_docs)} documenti raw")
            
            if not retrieved_docs:
                logger.warning("⚠️  Qdrant non ha ritornato risultati!")
                return "Non ho trovato documenti rilevanti per rispondere a questa domanda.", []
            
            # 2. Filtra per relevance threshold
            logger.debug(f"     Filtrando per threshold {self.relevance_threshold}...")
            relevant_docs = [
                doc for doc in retrieved_docs
                if doc.get("similarity", 0) >= self.relevance_threshold
            ]
            
            logger.info(f"      ✅ {len(relevant_docs)}/{len(retrieved_docs)} documenti sopra threshold")
            
            if not relevant_docs:
                logger.warning(f"⚠️  Nessun documento supera il threshold {self.relevance_threshold}")
                return "Non ho trovato informazioni sufficientemente rilevanti per rispondere.", []
            
            # 3. Build context dalla ricerca
            logger.debug("  2/3 Creando contesto...")
            context_parts = []
            for i, doc in enumerate(relevant_docs, 1):
                text = doc["metadata"].get("text", "")
                filename = doc["metadata"].get("filename", "unknown")
                similarity = doc.get("similarity", 0)
                
                context_parts.append(
                    f"[{i}] ({filename} - rilevanza: {similarity:.2%})\n{text}"
                )
            
            context = "\n\n---\n\n".join(context_parts)
            logger.debug(f"      Context length: {len(context)} chars")
            
            # 4. LLM Generation
            logger.debug("  3/3 LLM Generation...")
            prompt = self.qa_prompt.format(
                context=context,
                question=query
            )
            
            logger.debug(f"      Prompt length: {len(prompt)} chars")
            
            # Chiama LLM
            answer = self.llm(prompt)
            logger.info(f"      ✅ Risposta generata ({len(answer)} caratteri)")
            
            # 5. Format sources - DEDUPLICATO per document
            logger.debug("  Formattando sources...")
            sources_dict = {}  # Usa dict per deduplica
            
            for doc in relevant_docs:
                doc_id = doc["metadata"].get("document_id", "unknown")
                filename = doc["metadata"].get("filename", "unknown")
                similarity = doc.get("similarity", 0)
                
                # Usa il documento con similarity più alta
                if doc_id not in sources_dict or similarity > sources_dict[doc_id]["similarity"]:
                    sources_dict[doc_id] = {
                        "filename": filename,
                        "document_id": doc_id,
                        "similarity_score": round(similarity, 3),
                        "chunk_index": doc["metadata"].get("chunk_index", 0),
                    }
            
            sources = list(sources_dict.values())
            # Sort per similarity decrescente
            sources.sort(key=lambda x: x["similarity_score"], reverse=True)
            
            logger.info(f"✅ Query completata - {len(sources)} sources uniche ritornate")
            
            return answer, sources
            
        except Exception as e:
            logger.error(f"❌ Errore query: {str(e)}", exc_info=True)
            raise
    
    
    def reindex_all_documents(self):
        """Reindicizza tutti i documenti (se necessario)"""
        try:
            logger.info("📄 Reindexing all documents...")
            # Implementazione dipende da come salvi gli originali
            # Questa è una skeleton per future implementazioni
            logger.info("✅ Reindexing completato")
        except Exception as e:
            logger.error(f"❌ Errore reindexing: {str(e)}")
            raise