"""
Pydantic models per autenticazione e gestione utenti
"""

from pydantic import BaseModel, EmailStr, Field
from typing import Optional
from datetime import datetime


class LoginRequest(BaseModel):
    """Request per login"""
    username: str = Field(..., min_length=3, max_length=50)
    password: str = Field(..., min_length=6)


class UserInfo(BaseModel):
    """Informazioni utente (senza password)"""
    id: int
    username: str
    email: str
    role: str
    created_at: str
    last_login: Optional[str] = None


class LoginResponse(BaseModel):
    """Response per login"""
    access_token: str
    token_type: str = "bearer"
    user: UserInfo


class UserCreate(BaseModel):
    """Request per creare nuovo utente"""
    username: str = Field(..., min_length=3, max_length=50)
    email: EmailStr
    password: str = Field(..., min_length=6)
    role: str = Field(..., pattern="^(admin|super_user|user)$")


class UserUpdate(BaseModel):
    """Request per aggiornare utente"""
    role: Optional[str] = Field(None, pattern="^(admin|super_user|user)$")
    email: Optional[EmailStr] = None


class PasswordChange(BaseModel):
    """Request per cambiare password"""
    old_password: str = Field(..., min_length=6)
    new_password: str = Field(..., min_length=6)


class UserListResponse(BaseModel):
    """Response lista utenti"""
    users: list[UserInfo]
    total: int


class MessageResponse(BaseModel):
    """Response generica con messaggio"""
    message: str
