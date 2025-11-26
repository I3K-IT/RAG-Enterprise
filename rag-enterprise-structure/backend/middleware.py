"""
Middleware per autenticazione e autorizzazione
"""

from fastapi import Depends, HTTPException, status
from fastapi.security import HTTPBearer, HTTPAuthorizationCredentials
from typing import Optional
import logging

from auth import verify_token
from database import UserRole, db

logger = logging.getLogger(__name__)

# Security scheme
security = HTTPBearer()


class CurrentUser:
    """Rappresenta l'utente corrente autenticato"""

    def __init__(self, user_id: int, username: str, role: str):
        self.user_id = user_id
        self.username = username
        self.role = role

    def is_admin(self) -> bool:
        return self.role == UserRole.ADMIN

    def is_super_user(self) -> bool:
        return self.role == UserRole.SUPER_USER

    def is_user(self) -> bool:
        return self.role == UserRole.USER

    def can_upload(self) -> bool:
        """Può caricare documenti"""
        return self.role in [UserRole.ADMIN, UserRole.SUPER_USER]

    def can_delete(self) -> bool:
        """Può cancellare documenti"""
        return self.role in [UserRole.ADMIN, UserRole.SUPER_USER]

    def can_manage_users(self) -> bool:
        """Può gestire utenti"""
        return self.role == UserRole.ADMIN


async def get_current_user(
    credentials: HTTPAuthorizationCredentials = Depends(security)
) -> CurrentUser:
    """
    Dependency per ottenere l'utente corrente dal token JWT

    Raises:
        HTTPException: Se token invalido o utente non trovato
    """
    token = credentials.credentials

    # Verifica token
    payload = verify_token(token)

    if not payload:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Token invalido o scaduto",
            headers={"WWW-Authenticate": "Bearer"},
        )

    # Verifica che l'utente esista ancora nel database
    user = db.get_user_by_id(payload["user_id"])

    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Utente non trovato",
            headers={"WWW-Authenticate": "Bearer"},
        )

    return CurrentUser(
        user_id=user["id"],
        username=user["username"],
        role=user["role"]
    )


async def require_admin(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency che richiede ruolo ADMIN

    Raises:
        HTTPException: Se utente non è admin
    """
    if not current_user.is_admin():
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Permessi insufficienti: richiesto ruolo ADMIN"
        )

    return current_user


async def require_super_user(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency che richiede ruolo SUPER_USER o ADMIN

    Raises:
        HTTPException: Se utente non è super_user o admin
    """
    if not (current_user.is_super_user() or current_user.is_admin()):
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Permessi insufficienti: richiesto ruolo SUPER_USER o ADMIN"
        )

    return current_user


async def require_upload_permission(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency che richiede permesso di upload

    Raises:
        HTTPException: Se utente non ha permesso di upload
    """
    if not current_user.can_upload():
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Permessi insufficienti: non puoi caricare documenti"
        )

    return current_user


async def require_delete_permission(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency che richiede permesso di cancellazione

    Raises:
        HTTPException: Se utente non ha permesso di cancellazione
    """
    if not current_user.can_delete():
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Permessi insufficienti: non puoi cancellare documenti"
        )

    return current_user


# Optional: Dependency per route pubbliche con utente opzionale
async def get_current_user_optional(
    credentials: Optional[HTTPAuthorizationCredentials] = Depends(HTTPBearer(auto_error=False))
) -> Optional[CurrentUser]:
    """
    Dependency per ottenere utente corrente se presente (route pubbliche)

    Returns:
        CurrentUser se token valido, None altrimenti
    """
    if not credentials:
        return None

    try:
        return await get_current_user(credentials)
    except HTTPException:
        return None
