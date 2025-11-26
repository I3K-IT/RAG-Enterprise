"""
Sistema di autenticazione JWT
"""

import jwt
from datetime import datetime, timedelta
from typing import Optional, Dict
import os
import logging

logger = logging.getLogger(__name__)

# Configurazione JWT
SECRET_KEY = os.getenv("JWT_SECRET_KEY", "your-secret-key-change-in-production-please-2025")
ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_MINUTES = 480  # 8 ore


def create_access_token(data: dict, expires_delta: Optional[timedelta] = None) -> str:
    """
    Crea un JWT access token

    Args:
        data: Dati da includere nel token (user_id, username, role)
        expires_delta: Durata del token (default: 8 ore)

    Returns:
        Token JWT come stringa
    """
    to_encode = data.copy()

    if expires_delta:
        expire = datetime.utcnow() + expires_delta
    else:
        expire = datetime.utcnow() + timedelta(minutes=ACCESS_TOKEN_EXPIRE_MINUTES)

    to_encode.update({"exp": expire})

    encoded_jwt = jwt.encode(to_encode, SECRET_KEY, algorithm=ALGORITHM)
    return encoded_jwt


def decode_access_token(token: str) -> Optional[Dict]:
    """
    Decodifica e valida un JWT token

    Args:
        token: Token JWT da decodificare

    Returns:
        Payload del token se valido, None se invalido o scaduto
    """
    try:
        payload = jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM])
        return payload
    except jwt.ExpiredSignatureError:
        logger.warning("Token JWT scaduto")
        return None
    except jwt.InvalidTokenError as e:
        logger.warning(f"Token JWT invalido: {e}")
        return None


def verify_token(token: str) -> Optional[Dict]:
    """
    Verifica token e ritorna i dati utente

    Returns:
        Dict con user_id, username, role se valido, None altrimenti
    """
    payload = decode_access_token(token)

    if not payload:
        return None

    # Verifica che ci siano i campi necessari
    if "user_id" not in payload or "username" not in payload or "role" not in payload:
        return None

    return {
        "user_id": payload["user_id"],
        "username": payload["username"],
        "role": payload["role"]
    }


def create_user_token(user: Dict) -> str:
    """
    Crea token per un utente

    Args:
        user: Dict con dati utente (id, username, role)

    Returns:
        Token JWT
    """
    token_data = {
        "user_id": user["id"],
        "username": user["username"],
        "role": user["role"]
    }

    return create_access_token(token_data)
