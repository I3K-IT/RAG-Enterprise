"""
Middleware for authentication and authorization
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
    """Represents the current authenticated user"""

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
        """Can upload documents"""
        return self.role in [UserRole.ADMIN, UserRole.SUPER_USER]

    def can_delete(self) -> bool:
        """Can delete documents"""
        return self.role in [UserRole.ADMIN, UserRole.SUPER_USER]

    def can_manage_users(self) -> bool:
        """Can manage users"""
        return self.role == UserRole.ADMIN


async def get_current_user(
    credentials: HTTPAuthorizationCredentials = Depends(security)
) -> CurrentUser:
    """
    Dependency to get current user from JWT token

    Raises:
        HTTPException: If token is invalid or user not found
    """
    token = credentials.credentials

    # Verify token
    payload = verify_token(token)

    if not payload:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid or expired token",
            headers={"WWW-Authenticate": "Bearer"},
        )

    # Verify that the user still exists in the database
    user = db.get_user_by_id(payload["user_id"])

    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="User not found",
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
    Dependency that requires ADMIN role

    Raises:
        HTTPException: If user is not admin
    """
    if not current_user.is_admin():
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Insufficient permissions: ADMIN role required"
        )

    return current_user


async def require_super_user(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency that requires SUPER_USER or ADMIN role

    Raises:
        HTTPException: If user is not super_user or admin
    """
    if not (current_user.is_super_user() or current_user.is_admin()):
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Insufficient permissions: SUPER_USER or ADMIN role required"
        )

    return current_user


async def require_upload_permission(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency that requires upload permission

    Raises:
        HTTPException: If user doesn't have upload permission
    """
    if not current_user.can_upload():
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Insufficient permissions: you cannot upload documents"
        )

    return current_user


async def require_delete_permission(
    current_user: CurrentUser = Depends(get_current_user)
) -> CurrentUser:
    """
    Dependency that requires delete permission

    Raises:
        HTTPException: If user doesn't have delete permission
    """
    if not current_user.can_delete():
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Insufficient permissions: you cannot delete documents"
        )

    return current_user


# Optional: Dependency for public routes with optional user
async def get_current_user_optional(
    credentials: Optional[HTTPAuthorizationCredentials] = Depends(HTTPBearer(auto_error=False))
) -> Optional[CurrentUser]:
    """
    Dependency to get current user if present (public routes)

    Returns:
        CurrentUser if token valid, None otherwise
    """
    if not credentials:
        return None

    try:
        return await get_current_user(credentials)
    except HTTPException:
        return None
