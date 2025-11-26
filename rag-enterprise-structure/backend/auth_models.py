"""
Pydantic models for authentication and user management
"""

from pydantic import BaseModel, EmailStr, Field
from typing import Optional
from datetime import datetime


class LoginRequest(BaseModel):
    """Login request"""
    username: str = Field(..., min_length=3, max_length=50)
    password: str = Field(..., min_length=6)


class UserInfo(BaseModel):
    """User information (without password)"""
    id: int
    username: str
    email: str
    role: str
    created_at: str
    last_login: Optional[str] = None


class LoginResponse(BaseModel):
    """Login response"""
    access_token: str
    token_type: str = "bearer"
    user: UserInfo


class UserCreate(BaseModel):
    """Request to create new user"""
    username: str = Field(..., min_length=3, max_length=50)
    email: EmailStr
    password: str = Field(..., min_length=6)
    role: str = Field(..., pattern="^(admin|super_user|user)$")


class UserUpdate(BaseModel):
    """Request to update user"""
    role: Optional[str] = Field(None, pattern="^(admin|super_user|user)$")
    email: Optional[EmailStr] = None


class PasswordChange(BaseModel):
    """Request to change password"""
    old_password: str = Field(..., min_length=6)
    new_password: str = Field(..., min_length=6)


class UserListResponse(BaseModel):
    """User list response"""
    users: list[UserInfo]
    total: int


class MessageResponse(BaseModel):
    """Generic response with message"""
    message: str
