# Backup & Restore - Complete Guide

**Full system backup with 70+ cloud providers support via rclone.**

RAG Enterprise includes a built-in backup system that protects your entire deployment: database, uploaded documents, and vector store. Backups can be stored locally or automatically synced to any major cloud provider.

> **Why this matters**: A RAG system without backups is a ticking time bomb. One failed disk, one bad update, and months of indexed documents are gone. This feature makes RAG Enterprise production-ready for real deployments.

![Backup Management Panel](images/backup-panel.png)

---

## Table of Contents

- [Overview](#overview)
- [What Gets Backed Up](#what-gets-backed-up)
- [Quick Start](#quick-start)
- [Cloud Providers](#cloud-providers)
  - [Supported Providers](#supported-providers)
  - [Setup Examples](#setup-examples)
- [Manual Backup](#manual-backup)
- [Automatic Scheduling](#automatic-scheduling)
- [Restore](#restore)
- [Backup Archive Format](#backup-archive-format)
- [API Reference](#api-reference)
- [Environment Variables](#environment-variables)
- [Troubleshooting](#troubleshooting)

---

## Overview

The backup system has three core capabilities:

| Capability | Description |
|------------|-------------|
| **Local Backup** | Creates a compressed `.tar.gz` archive with all system data |
| **Cloud Sync** | Uploads backups to 70+ cloud providers via rclone |
| **Scheduled Backup** | Cron-based automatic backups with retention policies |

All backup operations are available from the **Admin Panel > Backup** tab and require admin privileges.

---

## What Gets Backed Up

Every backup includes three components:

| Component | Description | Method |
|-----------|-------------|--------|
| **SQLite Database** | Users, roles, sessions, settings | Safe online backup via SQLite backup API |
| **Uploaded Documents** | All PDFs, DOCX, TXT, etc. in `/app/uploads` | Full directory copy |
| **Qdrant Vector Store** | All embeddings and indexed data | Snapshot via Qdrant REST API |

Each component can be individually selected during restore.

---

## Quick Start

### 1. Create a Local Backup

Click **"Local Backup"** in the admin panel. Done. The system creates a timestamped archive in `/app/backups/`.

### 2. Add a Cloud Provider (Optional)

Go to **Cloud Providers > Add Provider**, select your provider type, enter credentials, and test the connection.

### 3. Schedule Automatic Backups (Recommended)

Set a cron expression (e.g., `0 2 * * *` for daily at 2:00 AM), select a cloud provider, set retention to keep the last N backups, and enable.

---

## Cloud Providers

### Supported Providers

RAG Enterprise supports **10 provider types** covering virtually every cloud storage service:

| Provider | Type ID | Required Configuration | Best For |
|----------|---------|----------------------|----------|
| **Mega** | `mega` | `user`, `pass` | Free 20GB, easy setup |
| **Amazon S3 / MinIO** | `s3` | `provider`, `access_key_id`, `secret_access_key` | Enterprise, self-hosted |
| **Google Drive** | `drive` | `token` | Google Workspace users |
| **Microsoft OneDrive** | `onedrive` | `token` | Microsoft 365 users |
| **Dropbox** | `dropbox` | `token` | Simple cloud storage |
| **WebDAV (Nextcloud/ownCloud)** | `webdav` | `url`, `user`, `pass` | Self-hosted clouds |
| **FTP / FTPS** | `ftp` | `host`, `user`, `pass` | Legacy infrastructure |
| **SFTP (SSH)** | `sftp` | `host`, `user` | Secure remote servers |
| **Backblaze B2** | `b2` | `account`, `key` | Cost-effective storage |
| **pCloud** | `pcloud` | `token` | European privacy |

> **Note**: Providers requiring OAuth tokens (Google Drive, OneDrive, Dropbox, pCloud) need you to generate the token externally using `rclone config` on your local machine, then paste it into the configuration form.

### Setup Examples

#### Mega (Easiest - No OAuth Required)

1. Go to **Admin > Backup > Cloud Providers**
2. Click **"Add Provider"**
3. Fill in:
   - **Name**: `my-mega` (your choice)
   - **Type**: Mega
   - **User**: your Mega email
   - **Password**: your Mega password
4. Click **"Add"**
5. Click **"Test"** to verify connection

#### Amazon S3 / MinIO

```
Name: my-s3
Type: Amazon S3 / MinIO
Provider: AWS (or Minio, Wasabi, etc.)
Access Key ID: AKIAIOSFODNN7EXAMPLE
Secret Access Key: wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
Region: eu-west-1 (optional)
Endpoint: https://minio.example.com (only for MinIO/compatible)
```

#### WebDAV (Nextcloud)

```
Name: my-nextcloud
Type: WebDAV (Nextcloud/ownCloud)
URL: https://cloud.example.com/remote.php/dav/files/username/
User: your-username
Password: your-password (or app password)
```

#### SFTP (Remote Server)

```
Name: my-server
Type: SFTP (SSH)
Host: backup.example.com
User: backupuser
Password: your-password
Port: 22 (optional, default 22)
```

#### Google Drive (Requires OAuth Token)

1. On your **local machine** (not the server), run:
   ```bash
   rclone config
   ```
2. Create a new remote of type `drive`
3. Follow the OAuth flow in your browser
4. Copy the generated token JSON
5. In the RAG Enterprise admin panel, create a provider:
   ```
   Name: my-gdrive
   Type: Google Drive
   Token: {"access_token":"...","token_type":"Bearer",...}
   ```

---

## Manual Backup

### From the UI

1. Go to **Admin > Backup**
2. Click **"Local Backup"** for a local-only backup
3. Or click **"Backup to [Provider]"** to backup and upload to cloud
4. Monitor progress in the **History** section

### From the API

```bash
# Local backup only
curl -X POST http://localhost:8000/api/admin/backup/run \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'

# Backup + upload to cloud
curl -X POST http://localhost:8000/api/admin/backup/run \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"provider": "my-mega", "remote_path": "rag-enterprise-backups"}'
```

### What Happens During Backup

1. SQLite database is backed up using the safe online backup API (no downtime)
2. All uploaded documents are copied
3. Qdrant creates a snapshot via REST API and downloads it
4. Everything is compressed into a single `.tar.gz` archive
5. If a cloud provider is specified, the archive is uploaded via rclone
6. History entry is logged

---

## Automatic Scheduling

### Cron Expression Format

```
┌───────────── minute (0-59)
│ ┌───────────── hour (0-23)
│ │ ┌───────────── day of month (1-31)
│ │ │ ┌───────────── month (1-12)
│ │ │ │ ┌───────────── day of week (0-6, Sun=0)
│ │ │ │ │
* * * * *
```

### Common Schedules

| Schedule | Cron Expression | Description |
|----------|----------------|-------------|
| Daily at 2 AM | `0 2 * * *` | Recommended for most deployments |
| Every 6 hours | `0 */6 * * *` | High-availability environments |
| Weekly (Sunday 3 AM) | `0 3 * * 0` | Low-change environments |
| Monthly (1st at 1 AM) | `0 1 1 * *` | Archival purposes |

### Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| **Cron** | `0 2 * * *` | When to run backups |
| **Provider** | None (local only) | Cloud provider for upload |
| **Remote Path** | `rag-enterprise-backups` | Folder name on cloud storage |
| **Retention** | 5 | Number of local backups to keep (1-100) |
| **Enabled** | false | Activate/deactivate schedule |

### Setup via UI

1. Go to **Admin > Backup > Automatic Schedule**
2. Set cron expression
3. Select cloud provider (optional)
4. Set retention (how many local backups to keep)
5. Check **"Enable"**
6. Click **"Save Schedule"**
7. Next run time is displayed after saving

### Setup via API

```bash
curl -X POST http://localhost:8000/api/admin/backup/schedule \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "cron": "0 2 * * *",
    "provider": "my-mega",
    "remote_path": "rag-enterprise-backups",
    "retention": 5,
    "enabled": true
  }'
```

### Scheduled Backup Flow

1. APScheduler triggers at the configured time
2. Full backup is created (database + documents + vectors)
3. If a provider is configured, backup is uploaded to cloud
4. Old local backups are cleaned up based on retention setting
5. Everything is logged to history

---

## Restore

### From the UI

1. Go to **Admin > Backup > Local Backups**
2. Find the backup you want to restore
3. Click **"Restore"**
4. Confirm the warning dialog (existing data will be overwritten)
5. Wait for restore to complete

### From Cloud Storage

1. Go to **Admin > Backup**
2. Find the backup on your cloud provider
3. Click **"Download"** to bring it to local storage
4. Then restore from the local copy

### From the API

```bash
curl -X POST http://localhost:8000/api/admin/backup/restore \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "filename": "rag_backup_20260227_020000.tar.gz",
    "restore_db": true,
    "restore_uploads": true,
    "restore_qdrant": true
  }'
```

### Selective Restore

You can restore individual components:

| Flag | What It Restores |
|------|-----------------|
| `restore_db` | SQLite database (users, sessions, settings) |
| `restore_uploads` | All uploaded documents |
| `restore_qdrant` | Vector database (embeddings, indexes) |

> **Warning**: Restore overwrites existing data. The system does not create a backup before restoring. Consider creating a manual backup first if you're unsure.

---

## Backup Archive Format

Each backup is a compressed tar.gz archive with the following structure:

```
rag_backup_YYYYMMDD_HHMMSS.tar.gz
└── rag_backup_YYYYMMDD_HHMMSS/
    ├── database/
    │   └── rag_users.db              # SQLite database
    ├── uploads/
    │   ├── document1.pdf             # Uploaded documents
    │   ├── document2.docx
    │   └── ...
    ├── qdrant/
    │   └── snapshot.tar              # Qdrant vector snapshot
    └── backup_metadata.json          # Backup metadata
```

### Metadata File

```json
{
  "version": "1.0.0",
  "timestamp": "20260227",
  "created_at": "2026-02-27T02:00:00.123456",
  "components": {
    "database": true,
    "uploads": true,
    "qdrant": true
  }
}
```

---

## API Reference

All endpoints require **admin authentication** (`Authorization: Bearer <token>`).

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/admin/backup/status` | System status (rclone installed, backup count) |
| `GET` | `/api/admin/backup/providers` | List configured + supported providers |
| `POST` | `/api/admin/backup/providers` | Add cloud provider |
| `DELETE` | `/api/admin/backup/providers/{name}` | Remove cloud provider |
| `POST` | `/api/admin/backup/providers/{name}/test` | Test provider connection |
| `POST` | `/api/admin/backup/run` | Trigger manual backup |
| `GET` | `/api/admin/backup/schedule` | Get current schedule |
| `POST` | `/api/admin/backup/schedule` | Set/update schedule |
| `GET` | `/api/admin/backup/history` | Backup execution history |
| `GET` | `/api/admin/backup/local` | List local backup files |
| `DELETE` | `/api/admin/backup/local/{filename}` | Delete local backup |
| `GET` | `/api/admin/backup/cloud/{provider}` | List cloud backups |
| `POST` | `/api/admin/backup/cloud/{provider}/download` | Download from cloud |
| `POST` | `/api/admin/backup/restore` | Restore from backup |

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `BACKUP_DIR` | `/app/backups` | Local backup storage directory |
| `UPLOAD_DIR` | `/app/uploads` | Document uploads directory |
| `DB_PATH` | `/app/data/rag_users.db` | SQLite database path |
| `QDRANT_HOST` | `qdrant` | Qdrant vector DB hostname |
| `QDRANT_PORT` | `6333` | Qdrant vector DB port |

---

## Troubleshooting

### "rclone not installed" in status

The Docker image needs to be rebuilt with rclone support:

```bash
cd rag-enterprise-structure
docker compose build --no-cache backend
docker compose up -d
```

### Cloud provider test fails

1. Double-check credentials (especially passwords and tokens)
2. For OAuth providers (Google Drive, OneDrive, Dropbox), ensure the token hasn't expired
3. Check network connectivity from the Docker container:
   ```bash
   docker compose exec backend rclone about my-provider: --config /app/backups/rclone.conf
   ```

### Qdrant snapshot fails during backup

The backup will still complete - documents and database are saved. The Qdrant error is logged in `qdrant/ERROR.txt` inside the archive. Common causes:
- Qdrant service temporarily unavailable
- Collection doesn't exist yet (no documents uploaded)

### Restore fails

- Ensure the backup file exists in `/app/backups/`
- Check that the archive isn't corrupted (try `tar tzf filename.tar.gz`)
- For Qdrant restore issues, verify the Qdrant service is running

### Scheduled backup not running

1. Verify the schedule is enabled: **Admin > Backup > Automatic Schedule**
2. Check that the cron expression is valid
3. Review backend logs: `docker compose logs backend | grep -i backup`
4. The schedule persists across restarts via `/app/backups/schedule_config.json`

---

## Security Notes

- All backup endpoints require **admin role** authentication
- Passwords for Mega, WebDAV, FTP, and SFTP are **encrypted** using rclone's obscure mechanism
- Archive extraction includes **path traversal protection** (prevents `../` attacks)
- Local backup deletion only allows files matching the `rag_backup_*` pattern
- Backup history retains the last **100 entries**

---

**Made with care for production deployments by [I3K Technologies](https://www.i3k.eu)**
