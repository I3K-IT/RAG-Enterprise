"""
Embeddings Service - Sentence-Transformers
Generates embeddings for texts (queries and documents)
"""

import logging
from typing import List, Union
import numpy as np
from sentence_transformers import SentenceTransformer
import torch

logger = logging.getLogger(__name__)


class EmbeddingsService:
    """
    Embeddings Service with Sentence-Transformers
    - Open-source models
    - Multilingual
    - GPU-accelerated
    """

    # Available models
    MODELS = {
        "all-MiniLM-L6-v2": {
            "description": "English, 22MB, veloce",
            "lang": "en",
            "dim": 384
        },
        "multilingual-MiniLM-L6-v2": {
            "description": "Multilingual, 61MB",
            "lang": "multilingual",
            "dim": 384
        },
        "all-mpnet-base-v2": {
            "description": "English, high quality, 430MB",
            "lang": "en",
            "dim": 768
        },
        "multilingual-e5-large": {
            "description": "Multilingual, high quality, 1.3GB",
            "lang": "multilingual",
            "dim": 1024
        },
        "deepseek-ai/deepseek-coder-6.7b-base": {
            "description": "DeepSeek Coder, high performance for code, 13GB",
            "lang": "multilingual",
            "dim": 4096
        },
        "BAAI/bge-large-en-v1.5": {
            "description": "BGE Large English, SOTA performance, 1.3GB",
            "lang": "en",
            "dim": 1024
        },
        "BAAI/bge-m3": {
            "description": "BGE M3 Multilingual, SOTA, dense+sparse+colbert, 2.3GB",
            "lang": "multilingual",
            "dim": 1024
        },
        "intfloat/e5-large-v2": {
            "description": "E5 Large v2, high performance multilingual, 1.3GB",
            "lang": "multilingual",
            "dim": 1024
        },
        "sentence-transformers/all-roberta-large-v1": {
            "description": "RoBERTa Large, high quality English, 1.3GB",
            "lang": "en",
            "dim": 1024
        }
    }
    
    
    def __init__(
        self,
        model_name: str = "BAAI/bge-m3",
        device: str = "cuda" if torch.cuda.is_available() else "cpu"
    ):
        """
        Initialize Embeddings Service

        Args:
            model_name: Sentence-Transformers model name
            device: 'cuda' or 'cpu'
        """
        self.model_name = model_name
        self.device = device
        
        if model_name not in self.MODELS:
            raise ValueError(f"Unknown model: {model_name}. Available: {list(self.MODELS.keys())}")
        
        logger.info(f"Loading embeddings model: {model_name} (device: {device})...")
        
        try:
            self.model = SentenceTransformer(model_name, device=device)
            self.embedding_dim = self.MODELS[model_name]["dim"]
            logger.info(f"✅ Model loaded (dim: {self.embedding_dim}, device: {device})")
        except Exception as e:
            logger.error(f"❌ Error loading model: {str(e)}")
            raise
    
    
    def embed_text(self, text: str) -> List[float]:
        """
        Generate embedding for single text

        Args:
            text: Text

        Returns:
            Embedding vector (list)
        """
        try:
            embedding = self.model.encode(
                text,
                convert_to_numpy=True,
                normalize_embeddings=True
            )
            return embedding.tolist()
        except Exception as e:
            logger.error(f"❌ Error embedding text: {str(e)}")
            raise
    
    
    def embed_texts(self, texts: List[str], batch_size: int = 32) -> List[List[float]]:
        """
        Generate embeddings for multiple texts

        Args:
            texts: List of texts
            batch_size: Batch size for processing

        Returns:
            List of embeddings
        """
        try:
            embeddings = self.model.encode(
                texts,
                batch_size=batch_size,
                convert_to_numpy=True,
                normalize_embeddings=True,
                show_progress_bar=True
            )

            # Convert to list of lists
            return embeddings.tolist()
            
        except Exception as e:
            logger.error(f"❌ Error embedding texts: {str(e)}")
            raise
    
    
    def similarity(self, text1: str, text2: str) -> float:
        """
        Calculate cosine similarity between two texts

        Returns:
            Value between 0 and 1
        """
        try:
            embeddings = self.model.encode([text1, text2])
            
            # Cosine similarity
            from sklearn.metrics.pairwise import cosine_similarity
            similarity = cosine_similarity([embeddings[0]], [embeddings[1]])[0][0]
            
            return float(similarity)
            
        except Exception as e:
            logger.error(f"❌ Error calculating similarity: {str(e)}")
            raise
    
    
    def get_embedding_dimension(self) -> int:
        """Embedding dimensionality"""
        return self.embedding_dim


    @staticmethod
    def list_available_models() -> dict:
        """List available models"""
        return EmbeddingsService.MODELS
