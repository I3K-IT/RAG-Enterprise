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
        chunk_size: int = 1000,
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
        """Template intelligente con controllo smart della memoria"""
        return """Sei un assistente AI con memoria conversazionale smart e intelligente.

LOGICA DI ELABORAZIONE SMART:
1. Leggi la domanda attuale con attenzione
2. Se contiene TUTTI i dati necessari (nomi, soggetti, riferimenti, date) → USALI DIRETTAMENTE senza consultare memoria
3. Se mancano dati (nome del soggetto, periodo temporale, etc.) → ALLORA controlla la cronologia per integrarli
4. Se trovi i dati mancanti in cronologia → usali in modo trasparente senza sottolineare che vengono da ricerca precedente
5. Se i dati non si trovano in nessun luogo (query + memoria + contesto) → ALLORA chiedi chiaramente cosa serve
6. Sii SEMPRE preciso nei calcoli matematici e temporali (anno attuale: 2025)
7. Non ripetere inutilmente informazioni già date
8. Sii conciso e diritto al punto

⚠️ REGOLA CRITICA ANTI-HALLUCINATION:
- Rispondi SOLO con informazioni presenti nel CONTESTO sottostante
- Se un dato (nome, codice fiscale, indirizzo, data) NON è nel contesto → di' "Non ho questa informazione nei documenti"
- NON inventare, NON ipotizzare, NON generare dati plausibili
- Meglio dire "Non so" che dare una risposta sbagliata

{history_section}

CONTESTO (da documenti):
{context}

DOMANDA:
{question}

RISPOSTA:"""
    
    
    def _format_history(self, history: List[Dict] = None) -> str:
        """
        Formatta la cronologia conversazionale - SOLO DOMANDE

        Fix anti-hallucination: Include solo le domande dell'utente,
        NON le risposte dell'assistant (che potrebbero essere sbagliate
        e creare loop di allucinazione)
        """
        if not history or len(history) == 0:
            return ""

        history_text = "DOMANDE PRECEDENTI DELL'UTENTE (per contesto):\n"
        for i, msg in enumerate(history[-3:], 1):  # Ultimi 3 scambi (ridotto da 5)
            user_msg = msg.get("user", "")
            if user_msg:  # Solo se c'è effettivamente una domanda
                history_text += f"{i}. {user_msg}\n"

        return history_text + "\n"
    
    
    def chunk_text(
        self,
        text: str,
        chunk_size: int = 1000,
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
        filename: str,
        document_type: str = "GENERIC_DOCUMENT",
        structured_fields: dict = None
    ):
        if structured_fields is None:
            structured_fields = {}
            
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
                    "document_type": document_type,
                    "structured_fields": str(structured_fields),
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
        temperature: float = 0.7,
        history: List[Dict] = None
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
                top_k=top_k,
                score_threshold=self.relevance_threshold  # ✅ FIX: Filtra a monte in Qdrant
            )

            logger.info(f"      ✅ Recuperati {len(retrieved_docs)} documenti (già filtrati da Qdrant con threshold={self.relevance_threshold})")

            # Log dettagliato dei documenti recuperati
            if retrieved_docs:
                logger.info("      📊 Similarity scores:")
                for i, doc in enumerate(retrieved_docs, 1):
                    filename = doc["metadata"].get("filename", "unknown")
                    similarity = doc.get("similarity", 0)
                    logger.info(f"         {i}. {filename}: {similarity:.3f} ({similarity:.1%})")
            
            if not retrieved_docs:
                logger.warning("⚠️  Qdrant non ha ritornato risultati sopra threshold!")
                logger.warning(f"⚠️  Possibili cause: threshold troppo alto ({self.relevance_threshold}) o documenti non rilevanti")
                return "Non ho trovato documenti rilevanti per rispondere a questa domanda.", []

            # ✅ Qdrant ha già filtrato per threshold, quindi retrieved_docs sono TUTTI rilevanti
            relevant_docs = retrieved_docs
            logger.info(f"      ✅ {len(relevant_docs)} documenti rilevanti (già filtrati da Qdrant)")
            
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
            
            # Formatta la storia conversazionale
            history_section = self._format_history(history)
            
            prompt = self.qa_prompt.format(
                history_section=history_section,
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
                if doc_id not in sources_dict or similarity > sources_dict[doc_id]["similarity_score"]:
                    sources_dict[doc_id] = {
                        "filename": filename,
                        "document_id": doc_id,
                        "similarity_score": round(similarity, 3),
                        "chunk_index": doc["metadata"].get("chunk_index", 0),
                        "text": doc["metadata"].get("text", "")
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