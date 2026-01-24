"""
Qdrant Connector - Vector Database Operations
Manages: insert, search, delete, collections
"""

import logging
from typing import List, Dict, Optional
from qdrant_client import QdrantClient
from qdrant_client.models import Distance, VectorParams, PointStruct, Filter, FieldCondition, MatchValue
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
    
    def __init__(self, host: str = "localhost", port: int = 6333, api_key: str = None):
        """
        Initialize connector

        Args:
            host: Qdrant host
            port: Qdrant port
            api_key: Qdrant API key (optional)
        """
        self.host = host
        self.port = port
        self.api_key = api_key
        self.client = None
        self.connected = False
    
    
    def connect(self):
        """Connection to Qdrant"""
        try:
            logger.info(f"Connecting to Qdrant: {self.host}:{self.port}...")
            
            client_params = {
                "host": self.host,
                "port": self.port,
                "timeout": 600
            }
            if self.api_key:
                client_params["api_key"] = self.api_key

            self.client = QdrantClient(**client_params)
            
            # Test connection
            self.client.get_collections()
            self.connected = True
            logger.info("✓ Qdrant connected")

            # Create or get collection
            self._initialize_collection()
            
        except Exception as e:
            logger.error(f"✗ Connection error: {str(e)}")
            raise
    
    
    def disconnect(self):
        """Disconnect from Qdrant"""
        try:
            if self.client:
                self.connected = False
                logger.info("✓ Qdrant disconnected")
        except Exception as e:
            logger.error(f"✗ Disconnect error: {str(e)}")
    
    
    def is_connected(self) -> bool:
        """Check connection status"""
        return self.connected
    
    
    def _initialize_collection(self):
        """Create collection if it doesn't exist"""
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
        """Insert vectors into collection with batching"""
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")
        
            if len(vectors) != len(metadatas):
                raise ValueError("Vectors and metadatas must have same length")
        
            logger.info(f"Inserting {len(vectors)} vectors...")
        
            inserted_ids = []
            BATCH_SIZE = 1000  # ← ADD BATCH

            # Insert in batch
            for batch_idx in range(0, len(vectors), BATCH_SIZE):
                batch_end = min(batch_idx + BATCH_SIZE, len(vectors))
                batch_vectors = vectors[batch_idx:batch_end]
                batch_metadatas = metadatas[batch_idx:batch_end]
            
                points = []
                for vector, metadata in zip(batch_vectors, batch_metadatas):
                    point_id = str(uuid.uuid4())
                    points.append(
                        PointStruct(
                            id=point_id,
                            vector=vector,
                            payload=metadata
                        )
                    )
                    inserted_ids.append(point_id)
            
                # Insert batch
                self.client.upsert(
                    collection_name=self.COLLECTION_NAME,
                    points=points,
                    wait=True
                )
            
                logger.info(f"  ✓ Batch {batch_idx}-{batch_end} inserted")
        
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
        Vector search

        Args:
            query_vector: Query embedding
            top_k: Number of results
            score_threshold: Similarity threshold

        Returns:
            List of results with metadata
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
        """Delete document from collection"""
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")

            logger.info(f"Deleting document: {document_id}")

            # Delete by filter usando API corretta
            self.client.delete(
                collection_name=self.COLLECTION_NAME,
                points_selector=Filter(
                    must=[
                        FieldCondition(
                            key="document_id",
                            match=MatchValue(value=document_id)
                        )
                    ]
                )
            )

            logger.info(f"✓ Document deleted: {document_id}")

        except Exception as e:
            logger.error(f"✗ Delete error: {str(e)}")
            raise
    
    
    def get_indexed_documents(self) -> List[Dict]:
        """Get list of indexed documents - with full pagination"""
        try:
            if not self.client:
                raise RuntimeError("Collection not initialized")

            # Scroll con pagination per ottenere TUTTI i punti
            all_points = []
            offset = None
            batch_size = 1000

            while True:
                points, next_offset = self.client.scroll(
                    collection_name=self.COLLECTION_NAME,
                    limit=batch_size,
                    offset=offset
                )

                all_points.extend(points)
                logger.info(f"📊 Fetched batch: {len(points)} points (total so far: {len(all_points)})")

                # If there's no next_offset or it's None, we're done
                if next_offset is None:
                    break

                offset = next_offset

            logger.info(f"✅ Retrieved {len(all_points)} total points from Qdrant")

            # Deduplicate by document_id and count chunks
            docs = {}
            for point in all_points:
                doc_id = point.payload.get("document_id")
                filename = point.payload.get("filename", "unknown")

                if doc_id not in docs:
                    docs[doc_id] = {
                        "document_id": doc_id,
                        "filename": filename,
                        "upload_date": point.payload.get("upload_date", ""),
                        "num_chunks": 0,
                        "status": "indexed"
                    }
                # Increment chunk count for this document
                docs[doc_id]["num_chunks"] += 1

            result = list(docs.values())
            logger.info(f"📋 Returning {len(result)} unique documents:")
            for doc in result:
                logger.info(f"   - {doc['filename']}: {doc['num_chunks']} chunks (ID: {doc['document_id'][:30]}...)")

            return result
            
        except Exception as e:
            logger.error(f"✗ Error getting documents: {str(e)}")
            return []
    
    
    def get_stats(self) -> Dict:
        """Collection statistics"""
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
