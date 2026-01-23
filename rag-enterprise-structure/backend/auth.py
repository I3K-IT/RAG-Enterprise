"""
JWT Authentication System
"""

import jwt
from datetime import datetime, timedelta
from typing import Optional, Dict
import os
import logging

logger = logging.getLogger(__name__)

# JWT Configuration
SECRET_KEY = os.getenv("JWT_SECRET_KEY", "")
ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_MINUTES = 480  # 8 hours

# Security: Generate a random key for development if not set
# In production, ALWAYS set JWT_SECRET_KEY in .env
if not SECRET_KEY:
    import secrets
    SECRET_KEY = secrets.token_hex(32)
    logger.warning("=" * 70)
    logger.warning("⚠️  JWT_SECRET_KEY not configured!")
    logger.warning("A random key has been generated for this session.")
    logger.warning("NOTE: Users will be logged out when the server restarts.")
    logger.warning("")
    logger.warning("For production, generate a permanent key:")
    logger.warning("  openssl rand -hex 32")
    logger.warning("Then add to .env: JWT_SECRET_KEY=<your-key>")
    logger.warning("=" * 70)
else:
    logger.info("✅ JWT_SECRET_KEY loaded from environment")


def create_access_token(data: dict, expires_delta: Optional[timedelta] = None) -> str:
    """
    Create a JWT access token

    Args:
        data: Data to include in the token (user_id, username, role)
        expires_delta: Token duration (default: 8 hours)

    Returns:
        JWT token as string
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
    Decode and validate a JWT token

    Args:
        token: JWT token to decode

    Returns:
        Token payload if valid, None if invalid or expired
    """
    try:
        payload = jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM])
        return payload
    except jwt.ExpiredSignatureError:
        logger.warning("JWT token expired")
        return None
    except jwt.InvalidTokenError as e:
        logger.warning(f"Invalid JWT token: {e}")
        return None


def verify_token(token: str) -> Optional[Dict]:
    """
    Verify token and return user data

    Returns:
        Dict with user_id, username, role if valid, None otherwise
    """
    payload = decode_access_token(token)

    if not payload:
        return None

    # Verify that necessary fields are present
    if "user_id" not in payload or "username" not in payload or "role" not in payload:
        return None

    return {
        "user_id": payload["user_id"],
        "username": payload["username"],
        "role": payload["role"]
    }


def create_user_token(user: Dict) -> str:
    """
    Create token for a user

    Args:
        user: Dict with user data (id, username, role)

    Returns:
        JWT token
    """
    token_data = {
        "user_id": user["id"],
        "username": user["username"],
        "role": user["role"]
    }

    return create_access_token(token_data)
