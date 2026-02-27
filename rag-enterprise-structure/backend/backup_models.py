"""
Pydantic models for Backup & Restore API
"""

from pydantic import BaseModel, Field
from typing import Optional, List, Dict


class BackupProviderCreate(BaseModel):
    """Request to add a cloud provider"""
    name: str = Field(..., min_length=1, max_length=50, description="Provider name (e.g. 'my-gdrive')")
    type: str = Field(..., description="Provider type (e.g. 'drive', 'mega', 's3')")
    config: Dict[str, str] = Field(..., description="Provider configuration key-value pairs")


class BackupProviderInfo(BaseModel):
    """Cloud provider info"""
    name: str
    type: str
    type_name: str


class BackupRunRequest(BaseModel):
    """Request to trigger a manual backup"""
    provider: Optional[str] = Field(None, description="Cloud provider to upload to (None = local only)")
    remote_path: str = Field("rag-enterprise-backups", description="Remote path/folder")


class BackupScheduleRequest(BaseModel):
    """Request to set backup schedule"""
    cron: str = Field(..., description="Cron expression: 'minute hour day month day_of_week'")
    provider: Optional[str] = Field(None, description="Cloud provider name")
    remote_path: str = Field("rag-enterprise-backups", description="Remote folder path")
    retention: int = Field(5, ge=1, le=100, description="Number of local backups to keep")
    enabled: bool = Field(True, description="Enable/disable the schedule")


class BackupRestoreRequest(BaseModel):
    """Request to restore from a backup"""
    filename: str = Field(..., description="Local backup filename to restore")
    restore_db: bool = Field(True, description="Restore SQLite database")
    restore_uploads: bool = Field(True, description="Restore uploaded documents")
    restore_qdrant: bool = Field(True, description="Restore Qdrant vector database")
