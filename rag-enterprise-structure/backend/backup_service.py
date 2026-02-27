"""
Backup Service - Core backup/restore logic with rclone cloud integration

Supports 70+ cloud providers via rclone:
- Google Drive, OneDrive, Mega, S3, Dropbox, Backblaze B2,
  pCloud, WebDAV (Nextcloud/ownCloud), FTP, SFTP, and more.
"""

import os
import json
import shutil
import sqlite3
import subprocess
import tarfile
import logging
from datetime import datetime
from typing import List, Dict, Optional

logger = logging.getLogger(__name__)

# Paths
BACKUP_DIR = os.getenv("BACKUP_DIR", "/app/backups")
RCLONE_CONFIG = os.path.join(BACKUP_DIR, "rclone.conf")
HISTORY_FILE = os.path.join(BACKUP_DIR, "history.json")
UPLOAD_DIR = os.getenv("UPLOAD_DIR", "/app/uploads")
DB_PATH = os.getenv("DB_PATH", "/app/data/rag_users.db")
QDRANT_HOST = os.getenv("QDRANT_HOST", "qdrant")
QDRANT_PORT = int(os.getenv("QDRANT_PORT", 6333))

# Supported cloud provider types and their required configuration
SUPPORTED_PROVIDERS = {
    "s3": {
        "name": "Amazon S3 / MinIO / Compatible",
        "required_fields": ["provider", "access_key_id", "secret_access_key"],
        "optional_fields": ["region", "endpoint", "bucket"]
    },
    "drive": {
        "name": "Google Drive",
        "required_fields": ["token"],
        "optional_fields": ["root_folder_id"]
    },
    "onedrive": {
        "name": "Microsoft OneDrive",
        "required_fields": ["token"],
        "optional_fields": ["drive_id", "drive_type"]
    },
    "mega": {
        "name": "Mega",
        "required_fields": ["user", "pass"],
        "optional_fields": []
    },
    "dropbox": {
        "name": "Dropbox",
        "required_fields": ["token"],
        "optional_fields": []
    },
    "b2": {
        "name": "Backblaze B2",
        "required_fields": ["account", "key"],
        "optional_fields": ["bucket"]
    },
    "pcloud": {
        "name": "pCloud",
        "required_fields": ["token"],
        "optional_fields": []
    },
    "webdav": {
        "name": "WebDAV (Nextcloud, ownCloud, etc.)",
        "required_fields": ["url", "user", "pass"],
        "optional_fields": []
    },
    "ftp": {
        "name": "FTP / FTPS",
        "required_fields": ["host", "user", "pass"],
        "optional_fields": ["port"]
    },
    "sftp": {
        "name": "SFTP (SSH)",
        "required_fields": ["host", "user"],
        "optional_fields": ["pass", "key_file", "port"]
    }
}


class BackupService:
    """Core backup service with rclone cloud integration"""

    def __init__(self):
        os.makedirs(BACKUP_DIR, exist_ok=True)
        self._ensure_history_file()

    def _ensure_history_file(self):
        if not os.path.exists(HISTORY_FILE):
            with open(HISTORY_FILE, 'w') as f:
                json.dump([], f)

    # ================================================================
    # BACKUP CREATION
    # ================================================================

    def create_backup(self) -> Dict:
        """Create a full backup archive of all RAG Enterprise data"""
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        backup_name = f"rag_backup_{timestamp}"
        temp_dir = os.path.join(BACKUP_DIR, f"_temp_{timestamp}")
        archive_path = os.path.join(BACKUP_DIR, f"{backup_name}.tar.gz")

        try:
            os.makedirs(temp_dir, exist_ok=True)
            logger.info(f"Creating backup: {backup_name}")

            # 1. SQLite database (safe online backup)
            self._backup_sqlite(temp_dir)

            # 2. Uploaded documents
            self._backup_uploads(temp_dir)

            # 3. Qdrant vector database snapshot
            self._backup_qdrant(temp_dir)

            # 4. Backup metadata
            self._save_backup_metadata(temp_dir, timestamp)

            # 5. Create compressed archive
            logger.info(f"Creating archive: {archive_path}")
            with tarfile.open(archive_path, "w:gz") as tar:
                tar.add(temp_dir, arcname=backup_name)

            archive_size = os.path.getsize(archive_path)
            logger.info(f"Backup created: {archive_path} ({archive_size / 1024 / 1024:.1f} MB)")

            return {
                "backup_name": backup_name,
                "archive_path": archive_path,
                "size_bytes": archive_size,
                "timestamp": timestamp
            }

        finally:
            if os.path.exists(temp_dir):
                shutil.rmtree(temp_dir)

    def _backup_sqlite(self, temp_dir: str):
        """Safe SQLite backup using the built-in backup API"""
        db_backup_dir = os.path.join(temp_dir, "database")
        os.makedirs(db_backup_dir, exist_ok=True)
        dest_path = os.path.join(db_backup_dir, "rag_users.db")

        if os.path.exists(DB_PATH):
            logger.info(f"Backing up SQLite: {DB_PATH}")
            src = sqlite3.connect(DB_PATH)
            dst = sqlite3.connect(dest_path)
            src.backup(dst)
            dst.close()
            src.close()
            logger.info("SQLite backup completed")
        else:
            logger.warning(f"SQLite DB not found at {DB_PATH}")

    def _backup_uploads(self, temp_dir: str):
        """Backup uploaded documents"""
        uploads_backup_dir = os.path.join(temp_dir, "uploads")

        if os.path.exists(UPLOAD_DIR) and os.listdir(UPLOAD_DIR):
            logger.info(f"Backing up uploads: {UPLOAD_DIR}")
            shutil.copytree(UPLOAD_DIR, uploads_backup_dir)
            file_count = len(os.listdir(uploads_backup_dir))
            logger.info(f"Uploads backup completed: {file_count} files")
        else:
            os.makedirs(uploads_backup_dir, exist_ok=True)
            logger.info("No uploads to backup")

    def _backup_qdrant(self, temp_dir: str):
        """Backup Qdrant via its REST snapshot API"""
        qdrant_backup_dir = os.path.join(temp_dir, "qdrant")
        os.makedirs(qdrant_backup_dir, exist_ok=True)

        try:
            import requests

            qdrant_url = f"http://{QDRANT_HOST}:{QDRANT_PORT}"
            collection_name = "rag_documents"

            # Create snapshot
            logger.info(f"Creating Qdrant snapshot for '{collection_name}'...")
            resp = requests.post(
                f"{qdrant_url}/collections/{collection_name}/snapshots",
                timeout=300
            )

            if resp.status_code != 200:
                logger.error(f"Qdrant snapshot failed: {resp.status_code} - {resp.text}")
                return

            snapshot_info = resp.json()["result"]
            snapshot_name = snapshot_info["name"]
            logger.info(f"Snapshot created: {snapshot_name}")

            # Download snapshot
            logger.info("Downloading Qdrant snapshot...")
            download_resp = requests.get(
                f"{qdrant_url}/collections/{collection_name}/snapshots/{snapshot_name}",
                stream=True,
                timeout=600
            )

            snapshot_path = os.path.join(qdrant_backup_dir, snapshot_name)
            with open(snapshot_path, 'wb') as f:
                for chunk in download_resp.iter_content(chunk_size=8192):
                    f.write(chunk)

            logger.info(f"Qdrant snapshot downloaded: {os.path.getsize(snapshot_path) / 1024 / 1024:.1f} MB")

            # Cleanup remote snapshot to save disk space
            requests.delete(
                f"{qdrant_url}/collections/{collection_name}/snapshots/{snapshot_name}",
                timeout=30
            )

        except Exception as e:
            logger.error(f"Qdrant backup error: {e}")
            with open(os.path.join(qdrant_backup_dir, "ERROR.txt"), 'w') as f:
                f.write(f"Qdrant backup failed: {str(e)}\n")

    def _save_backup_metadata(self, temp_dir: str, timestamp: str):
        """Save metadata about the backup contents"""
        qdrant_dir = os.path.join(temp_dir, "qdrant")
        metadata = {
            "version": "1.0.0",
            "timestamp": timestamp,
            "created_at": datetime.now().isoformat(),
            "components": {
                "database": os.path.exists(os.path.join(temp_dir, "database", "rag_users.db")),
                "uploads": bool(os.listdir(os.path.join(temp_dir, "uploads"))) if os.path.exists(os.path.join(temp_dir, "uploads")) else False,
                "qdrant": any(
                    not f.startswith("ERROR") for f in os.listdir(qdrant_dir)
                ) if os.path.exists(qdrant_dir) else False
            }
        }

        with open(os.path.join(temp_dir, "backup_metadata.json"), 'w') as f:
            json.dump(metadata, f, indent=2)

    # ================================================================
    # CLOUD UPLOAD / DOWNLOAD (rclone)
    # ================================================================

    def upload_to_cloud(self, archive_path: str, provider_name: str, remote_path: str = "rag-enterprise-backups") -> Dict:
        """Upload backup archive to cloud via rclone"""
        if not os.path.exists(archive_path):
            raise FileNotFoundError(f"Archive not found: {archive_path}")

        if not self._is_rclone_installed():
            raise RuntimeError("rclone is not installed. Rebuild the Docker image to enable cloud backup.")

        logger.info(f"Uploading {os.path.basename(archive_path)} to {provider_name}:{remote_path}")

        result = subprocess.run(
            [
                "rclone", "copy",
                archive_path,
                f"{provider_name}:{remote_path}",
                "--config", RCLONE_CONFIG,
                "--progress",
                "--transfers", "1",
                "-v"
            ],
            capture_output=True,
            text=True,
            timeout=3600
        )

        if result.returncode != 0:
            logger.error(f"rclone upload failed: {result.stderr}")
            raise RuntimeError(f"Cloud upload failed: {result.stderr}")

        logger.info(f"Upload completed to {provider_name}:{remote_path}")

        return {
            "provider": provider_name,
            "remote_path": remote_path,
            "filename": os.path.basename(archive_path),
            "status": "uploaded"
        }

    def download_from_cloud(self, provider_name: str, remote_file: str, remote_path: str = "rag-enterprise-backups") -> str:
        """Download a backup archive from cloud"""
        local_path = os.path.join(BACKUP_DIR, remote_file)

        result = subprocess.run(
            [
                "rclone", "copy",
                f"{provider_name}:{remote_path}/{remote_file}",
                BACKUP_DIR,
                "--config", RCLONE_CONFIG,
                "--progress",
                "-v"
            ],
            capture_output=True,
            text=True,
            timeout=3600
        )

        if result.returncode != 0:
            raise RuntimeError(f"Cloud download failed: {result.stderr}")

        return local_path

    def list_cloud_backups(self, provider_name: str, remote_path: str = "rag-enterprise-backups") -> List[Dict]:
        """List backup files on cloud storage"""
        result = subprocess.run(
            [
                "rclone", "lsjson",
                f"{provider_name}:{remote_path}",
                "--config", RCLONE_CONFIG
            ],
            capture_output=True,
            text=True,
            timeout=60
        )

        if result.returncode != 0:
            raise RuntimeError(f"Failed to list cloud backups: {result.stderr}")

        files = json.loads(result.stdout)
        return [
            {
                "name": f["Name"],
                "size_bytes": f["Size"],
                "modified": f["ModTime"]
            }
            for f in files
            if f["Name"].startswith("rag_backup_") and f["Name"].endswith(".tar.gz")
        ]

    # ================================================================
    # PROVIDER MANAGEMENT
    # ================================================================

    def list_providers(self) -> List[Dict]:
        """List configured rclone remotes"""
        if not self._is_rclone_installed() or not os.path.exists(RCLONE_CONFIG):
            return []

        result = subprocess.run(
            ["rclone", "listremotes", "--config", RCLONE_CONFIG],
            capture_output=True,
            text=True,
            timeout=10
        )

        if result.returncode != 0:
            return []

        providers = []
        for line in result.stdout.strip().split("\n"):
            if line.strip():
                name = line.strip().rstrip(":")
                provider_type = self._get_provider_type(name)
                providers.append({
                    "name": name,
                    "type": provider_type,
                    "type_name": SUPPORTED_PROVIDERS.get(provider_type, {}).get("name", provider_type)
                })

        return providers

    def _get_provider_type(self, name: str) -> str:
        """Get the type of a configured rclone remote"""
        result = subprocess.run(
            ["rclone", "config", "show", name, "--config", RCLONE_CONFIG],
            capture_output=True,
            text=True,
            timeout=10
        )

        for line in result.stdout.split("\n"):
            if line.strip().startswith("type"):
                return line.split("=")[1].strip()

        return "unknown"

    def add_provider(self, name: str, provider_type: str, config: Dict) -> Dict:
        """Add a new rclone remote (cloud provider)"""
        if provider_type not in SUPPORTED_PROVIDERS:
            raise ValueError(
                f"Unsupported provider type: {provider_type}. "
                f"Supported: {list(SUPPORTED_PROVIDERS.keys())}"
            )

        if not self._is_rclone_installed():
            raise RuntimeError("rclone is not installed")

        cmd = [
            "rclone", "config", "create",
            name, provider_type,
            "--config", RCLONE_CONFIG,
            "--non-interactive"
        ]

        # Obscure passwords for providers that need it
        config_copy = config.copy()
        if provider_type in ("mega", "webdav", "ftp", "sftp") and "pass" in config_copy:
            obscure_result = subprocess.run(
                ["rclone", "obscure", config_copy["pass"]],
                capture_output=True, text=True, timeout=10
            )
            if obscure_result.returncode == 0:
                config_copy["pass"] = obscure_result.stdout.strip()

        for key, value in config_copy.items():
            cmd.extend([key, str(value)])

        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)

        if result.returncode != 0:
            raise RuntimeError(f"Failed to add provider: {result.stderr}")

        logger.info(f"Provider added: {name} ({provider_type})")

        return {
            "name": name,
            "type": provider_type,
            "type_name": SUPPORTED_PROVIDERS[provider_type]["name"],
            "status": "configured"
        }

    def remove_provider(self, name: str) -> bool:
        """Remove an rclone remote"""
        result = subprocess.run(
            ["rclone", "config", "delete", name, "--config", RCLONE_CONFIG],
            capture_output=True, text=True, timeout=10
        )

        if result.returncode != 0:
            raise RuntimeError(f"Failed to remove provider: {result.stderr}")

        logger.info(f"Provider removed: {name}")
        return True

    def test_provider(self, name: str) -> Dict:
        """Test connection to a cloud provider"""
        result = subprocess.run(
            ["rclone", "about", f"{name}:", "--config", RCLONE_CONFIG, "--json"],
            capture_output=True, text=True, timeout=30
        )

        if result.returncode != 0:
            return {"status": "error", "message": result.stderr.strip()}

        try:
            info = json.loads(result.stdout)
            return {
                "status": "ok",
                "total": info.get("total"),
                "used": info.get("used"),
                "free": info.get("free")
            }
        except json.JSONDecodeError:
            return {"status": "ok", "message": "Connection successful"}

    def get_supported_providers(self) -> Dict:
        """Return list of supported provider types with their config requirements"""
        return SUPPORTED_PROVIDERS

    # ================================================================
    # RESTORE
    # ================================================================

    def restore_from_backup(
        self,
        archive_path: str,
        restore_db: bool = True,
        restore_uploads: bool = True,
        restore_qdrant: bool = True
    ) -> Dict:
        """Restore system data from a backup archive"""
        if not os.path.exists(archive_path):
            raise FileNotFoundError(f"Archive not found: {archive_path}")

        temp_dir = os.path.join(BACKUP_DIR, "_restore_temp")

        try:
            logger.info(f"Extracting backup: {archive_path}")
            if os.path.exists(temp_dir):
                shutil.rmtree(temp_dir)
            os.makedirs(temp_dir, exist_ok=True)

            with tarfile.open(archive_path, "r:gz") as tar:
                # Security: prevent path traversal
                for member in tar.getmembers():
                    if member.name.startswith("/") or ".." in member.name:
                        raise ValueError(f"Unsafe path in archive: {member.name}")
                tar.extractall(temp_dir)

            extracted_dirs = os.listdir(temp_dir)
            if not extracted_dirs:
                raise ValueError("Empty backup archive")

            backup_root = os.path.join(temp_dir, extracted_dirs[0])

            metadata_path = os.path.join(backup_root, "backup_metadata.json")
            if os.path.exists(metadata_path):
                with open(metadata_path, 'r') as f:
                    metadata = json.load(f)
                logger.info(f"Restoring backup from: {metadata.get('created_at', 'unknown')}")

            restored = {}

            # 1. Restore SQLite DB
            if restore_db:
                db_src = os.path.join(backup_root, "database", "rag_users.db")
                if os.path.exists(db_src):
                    logger.info("Restoring SQLite database...")
                    shutil.copy2(db_src, DB_PATH)
                    restored["database"] = True
                    logger.info("SQLite database restored")
                else:
                    restored["database"] = False
                    logger.warning("No database found in backup")

            # 2. Restore uploads
            if restore_uploads:
                uploads_src = os.path.join(backup_root, "uploads")
                if os.path.exists(uploads_src) and os.listdir(uploads_src):
                    logger.info("Restoring uploads...")
                    for item in os.listdir(uploads_src):
                        src = os.path.join(uploads_src, item)
                        dst = os.path.join(UPLOAD_DIR, item)
                        if os.path.isfile(src):
                            shutil.copy2(src, dst)
                    restored["uploads"] = True
                    restored["uploads_count"] = len(os.listdir(uploads_src))
                    logger.info(f"Uploads restored: {restored['uploads_count']} files")
                else:
                    restored["uploads"] = False

            # 3. Restore Qdrant
            if restore_qdrant:
                qdrant_src = os.path.join(backup_root, "qdrant")
                if os.path.exists(qdrant_src):
                    snapshots = [f for f in os.listdir(qdrant_src) if not f.startswith("ERROR")]
                    if snapshots:
                        restored["qdrant"] = self._restore_qdrant_snapshot(
                            os.path.join(qdrant_src, snapshots[0])
                        )
                    else:
                        restored["qdrant"] = False
                else:
                    restored["qdrant"] = False

            return {"status": "completed", "restored": restored}

        finally:
            if os.path.exists(temp_dir):
                shutil.rmtree(temp_dir)

    def _restore_qdrant_snapshot(self, snapshot_path: str) -> bool:
        """Restore Qdrant collection from a snapshot file"""
        try:
            import requests

            qdrant_url = f"http://{QDRANT_HOST}:{QDRANT_PORT}"
            collection_name = "rag_documents"

            logger.info(f"Restoring Qdrant snapshot: {snapshot_path}")

            with open(snapshot_path, 'rb') as f:
                resp = requests.post(
                    f"{qdrant_url}/collections/{collection_name}/snapshots/upload",
                    files={"snapshot": f},
                    params={"priority": "snapshot"},
                    timeout=600
                )

            if resp.status_code == 200:
                logger.info("Qdrant snapshot restored successfully")
                return True
            else:
                logger.error(f"Qdrant restore failed: {resp.status_code} - {resp.text}")
                return False

        except Exception as e:
            logger.error(f"Qdrant restore error: {e}")
            return False

    # ================================================================
    # BACKUP HISTORY
    # ================================================================

    def log_backup(self, entry: Dict):
        """Add an entry to the backup history log"""
        history = self.get_history()
        history.append(entry)

        # Keep last 100 entries
        if len(history) > 100:
            history = history[-100:]

        with open(HISTORY_FILE, 'w') as f:
            json.dump(history, f, indent=2)

    def get_history(self) -> List[Dict]:
        """Get backup history"""
        self._ensure_history_file()
        with open(HISTORY_FILE, 'r') as f:
            return json.load(f)

    # ================================================================
    # LOCAL BACKUP MANAGEMENT
    # ================================================================

    def list_local_backups(self) -> List[Dict]:
        """List local backup archives"""
        backups = []
        for f in sorted(os.listdir(BACKUP_DIR), reverse=True):
            if f.startswith("rag_backup_") and f.endswith(".tar.gz"):
                path = os.path.join(BACKUP_DIR, f)
                backups.append({
                    "name": f,
                    "path": path,
                    "size_bytes": os.path.getsize(path),
                    "created": datetime.fromtimestamp(os.path.getmtime(path)).isoformat()
                })
        return backups

    def delete_local_backup(self, filename: str) -> bool:
        """Delete a local backup archive"""
        path = os.path.join(BACKUP_DIR, filename)
        if os.path.exists(path) and filename.startswith("rag_backup_"):
            os.remove(path)
            return True
        return False

    def cleanup_old_backups(self, keep_last: int = 5):
        """Remove old local backups, keeping only the last N"""
        backups = self.list_local_backups()
        if len(backups) > keep_last:
            for backup in backups[keep_last:]:
                self.delete_local_backup(backup["name"])
                logger.info(f"Cleaned up old backup: {backup['name']}")

    # ================================================================
    # UTILITIES
    # ================================================================

    def _is_rclone_installed(self) -> bool:
        """Check if rclone binary is available"""
        try:
            result = subprocess.run(
                ["rclone", "version"],
                capture_output=True, text=True, timeout=5
            )
            return result.returncode == 0
        except (FileNotFoundError, subprocess.TimeoutExpired):
            return False

    def get_status(self) -> Dict:
        """Get backup system status overview"""
        return {
            "rclone_installed": self._is_rclone_installed(),
            "backup_dir": BACKUP_DIR,
            "local_backups": len(self.list_local_backups()),
            "configured_providers": len(self.list_providers()),
            "history_entries": len(self.get_history())
        }


# Global instance
backup_service = BackupService()
