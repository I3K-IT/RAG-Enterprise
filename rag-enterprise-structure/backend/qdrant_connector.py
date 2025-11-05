"""
Qdrant Connector - Vector Database Operations
Gestisce: insert, search, delete, collections
"""

import logging
from typing import List, Dict, Optional
from qdrant_client import QdrantClient
from qdrant_client.models import Distance, VectorParams, PointStruct
import uuid

logger = logging.getLogger(__name__)


class QdrantConnector:
    """
    Connector per Qdrant Vector Database
    - Connection management
    - Collection operations
    - Vector search
    """
    
    COLLECTION_NAME = "rag_documents"
    VECTOR_SIZE = 1024  # BAAI/bge-m3
    
    def __init__(self, host: str = "localhost", port: int = 6333):
        """
        Inizializza connector
        
        Args:
            host: Qdrant host
            port: Qdrant port
        """
        self.host = host
        self.port = port
        self.client = None
        self.connected = False
    
    
    def connect(self):
        """Connessione a Qdrant"""
        try:
            logger.info(f"Connecting to Qdrant: {self.host}:{self.port}...")
            
            self.client = QdrantClient(
                host=self.host,
                port=self.port
            )
            
            # Test connection
            self.client.get_collections()
            self.connected = True
            logger.info("✓ Qdrant connected")
            
            # Crea o ottieni collection
            self._initialize_collection()
            
        except Exception as e:
            logger.error(f"✗ Connection error: {str(e)}")
            raise
    
    
    def disconnect(self):
        """Disconnessione"""
        try:
            if self.client:
                self.connected = False
                logger.info("✓ Qdrant disconnected")
        except Exception as e:
            logger.error(f"✗ Disconnect error: {str(e)}")
    
    
    def is_connected(self) -> bool:
        """Check connessione"""
        return self.connected
    
    
    def _initialize_collection(self):
        """Crea collection se non esiste"""
        try:
            collections = self.client.get_collections()
            collection_names = [c.name for c in collections.collections]
            
            if self.COLLECTION_NAME in collection_names:
                logger.info(f"Collection '{self.COLLECTION_NAME}' exists")
            else:
                logger.info(f"Creating collection '{self.COLLECTION_NAME}'...")
                self.client.create_collection(
                    collection_name=self.COLLECTION_NAME,
                    vectors_config=VectorParams(
                        size=self.VECTOR_SIZE,
                        distance=Distance.COSINE
                    )
                )
                logger.info(f"✓ Collection created")
        
        except Exception as e:
            logger.error(f"✗ Collection initialization error: {str(e)}")
            raise
    
    
    def insert_vectors(
        self,
        vectors: List[List[float]],
        metadatas: List[Dict]
    ) -> List[str]:
        """
        Inserisci vettori in collection
        
        Args:
            vectors: Lista di embeddings
            metadatas: Lista di metadata (uno per vettore)
            
        Returns:
            IDs inseriti
        """
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")
            
            if len(vectors) != len(metadatas):
                raise ValueError("Vectors and metadatas must have same length")
            
            logger.info(f"Inserting {len(vectors)} vectors...")
            
            # Prepara punti
            points = []
            inserted_ids = []
            
            for i, (vector, metadata) in enumerate(zip(vectors, metadatas)):
                point_id = str(uuid.uuid4())
                points.append(
                    PointStruct(
                        id=point_id,
                        vector=vector,
                        payload=metadata
                    )
                )
                inserted_ids.append(point_id)
            
            # Insert
            self.client.upsert(
                collection_name=self.COLLECTION_NAME,
                points=points
            )
            
            logger.info(f"✓ Inserted {len(inserted_ids)} vectors")
            return inserted_ids
            
        except Exception as e:
            logger.error(f"✗ Insert error: {str(e)}")
            raise
    
    
    def search(
        self,
        query_vector: List[float],
        top_k: int = 5,
        score_threshold: Optional[float] = None
    ) -> List[Dict]:
        """
        Ricerca vettoriale
        
        Args:
            query_vector: Query embedding
            top_k: Numero risultati
            score_threshold: Soglia di similarità
            
        Returns:
            Lista di risultati con metadata
        """
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")
            
            logger.info(f"Searching for top {top_k} results...")
            
            results = self.client.search(
                collection_name=self.COLLECTION_NAME,
                query_vector=query_vector,
                limit=top_k,
                score_threshold=score_threshold
            )
            
            # Parse results
            search_results = []
            for hit in results:
                search_results.append({
                    "id": hit.id,
                    "similarity": float(hit.score),
                    "metadata": hit.payload
                })
            
            logger.info(f"✓ Found {len(search_results)} results")
            return search_results
            
        except Exception as e:
            logger.error(f"✗ Search error: {str(e)}")
            raise
    
    
    def delete_document(self, document_id: str):
        """Elimina documento da collection"""
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")
            
            logger.info(f"Deleting document: {document_id}")
            
            # Delete by filter
            self.client.delete(
                collection_name=self.COLLECTION_NAME,
                points_selector={
                    "filter": {
                        "must": [
                            {
                                "key": "document_id",
                                "match": {
                                    "value": document_id
                                }
                            }
                        ]
                    }
                }
            )
            
            logger.info(f"✓ Document deleted")
            
        except Exception as e:
            logger.error(f"✗ Delete error: {str(e)}")
            raise
    
    
    def get_indexed_documents(self) -> List[Dict]:
        """Ottieni lista documenti indicizzati"""
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")
            
            # Scroll per ottenere tutti i punti
            points, _ = self.client.scroll(
                collection_name=self.COLLECTION_NAME,
                limit=1000
            )
            
            # Deduplicaci per document_id
            docs = {}
            for point in points:
                doc_id = point.payload.get("document_id")
                if doc_id not in docs:
                    docs[doc_id] = {
                        "document_id": doc_id,
                        "filename": point.payload.get("filename"),
                        "upload_date": point.payload.get("upload_date", "")
                    }
            
            return list(docs.values())
            
        except Exception as e:
            logger.error(f"✗ Error getting documents: {str(e)}")
            return []
    
    
    def get_stats(self) -> Dict:
        """Statistiche collection"""
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")
            
            collection_info = self.client.get_collection(self.COLLECTION_NAME)
            
            return {
                "collection_name": self.COLLECTION_NAME,
                "num_vectors": collection_info.points_count,
                "vector_size": self.VECTOR_SIZE,
                "status": "healthy" if self.connected else "disconnected"
            }
            
        except Exception as e:
            logger.error(f"✗ Error getting stats: {str(e)}")
            return {"status": "error", "message": str(e)}
