"""
Backup Scheduler - APScheduler integration for automated backups

Provides cron-based scheduling for automatic backups with
optional cloud upload and retention management.
"""

import json
import os
import logging
from datetime import datetime
from typing import Optional, Dict

from apscheduler.schedulers.background import BackgroundScheduler
from apscheduler.triggers.cron import CronTrigger

logger = logging.getLogger(__name__)

SCHEDULE_CONFIG_FILE = os.path.join(
    os.getenv("BACKUP_DIR", "/app/backups"),
    "schedule_config.json"
)


class BackupScheduler:
    """Manages scheduled backup execution using APScheduler"""

    def __init__(self, backup_service):
        self.backup_service = backup_service
        self.scheduler = BackgroundScheduler()
        self._config = self._load_config()

    def start(self):
        """Start the scheduler (call on app startup)"""
        if self._config.get("enabled"):
            self._apply_schedule()
        self.scheduler.start()
        logger.info("Backup scheduler started")

    def stop(self):
        """Stop the scheduler (call on app shutdown)"""
        self.scheduler.shutdown(wait=False)
        logger.info("Backup scheduler stopped")

    def set_schedule(
        self,
        cron_expression: str,
        provider: Optional[str] = None,
        remote_path: str = "rag-enterprise-backups",
        retention: int = 5,
        enabled: bool = True
    ) -> Dict:
        """
        Set or update the backup schedule.

        cron_expression format: "minute hour day month day_of_week"
        Examples:
            "0 2 * * *"     - Daily at 2:00 AM
            "0 3 * * 0"     - Weekly on Sunday at 3:00 AM
            "0 1 1 * *"     - Monthly on the 1st at 1:00 AM
            "0 */6 * * *"   - Every 6 hours
        """
        self._config = {
            "enabled": enabled,
            "cron": cron_expression,
            "provider": provider,
            "remote_path": remote_path,
            "retention": retention,
            "updated_at": datetime.now().isoformat()
        }

        self._save_config()

        if enabled:
            self._apply_schedule()
        else:
            self._remove_schedule()

        return self.get_schedule()

    def get_schedule(self) -> Dict:
        """Get current schedule configuration with next run time"""
        config = self._config.copy()

        job = self.scheduler.get_job("backup_job")
        if job and job.next_run_time:
            config["next_run"] = job.next_run_time.isoformat()
        else:
            config["next_run"] = None

        return config

    def disable_schedule(self):
        """Disable the backup schedule"""
        self._config["enabled"] = False
        self._save_config()
        self._remove_schedule()

    def _apply_schedule(self):
        """Apply the current cron schedule to APScheduler"""
        self._remove_schedule()

        cron = self._config.get("cron", "0 2 * * *")
        parts = cron.split()

        if len(parts) != 5:
            raise ValueError(
                f"Invalid cron expression: {cron}. "
                "Expected 5 fields: minute hour day month day_of_week"
            )

        trigger = CronTrigger(
            minute=parts[0],
            hour=parts[1],
            day=parts[2],
            month=parts[3],
            day_of_week=parts[4]
        )

        self.scheduler.add_job(
            self._execute_scheduled_backup,
            trigger=trigger,
            id="backup_job",
            name="RAG Enterprise Scheduled Backup",
            replace_existing=True
        )

        logger.info(f"Backup schedule applied: {cron}")

    def _remove_schedule(self):
        """Remove the scheduled backup job"""
        try:
            self.scheduler.remove_job("backup_job")
        except Exception:
            pass

    def _execute_scheduled_backup(self):
        """Execute a scheduled backup (called by APScheduler)"""
        logger.info("=" * 60)
        logger.info("SCHEDULED BACKUP STARTED")
        logger.info("=" * 60)

        start_time = datetime.now()
        entry = {
            "type": "scheduled",
            "started_at": start_time.isoformat(),
            "cron": self._config.get("cron"),
            "provider": self._config.get("provider")
        }

        try:
            # Create backup
            result = self.backup_service.create_backup()
            entry["backup_name"] = result["backup_name"]
            entry["size_bytes"] = result["size_bytes"]

            # Upload to cloud if provider is configured
            provider = self._config.get("provider")
            remote_path = self._config.get("remote_path", "rag-enterprise-backups")

            if provider:
                upload_result = self.backup_service.upload_to_cloud(
                    result["archive_path"],
                    provider,
                    remote_path
                )
                entry["cloud_upload"] = upload_result

            # Cleanup old local backups
            retention = self._config.get("retention", 5)
            self.backup_service.cleanup_old_backups(keep_last=retention)

            entry["status"] = "success"
            entry["duration_seconds"] = (datetime.now() - start_time).total_seconds()

            logger.info(f"Scheduled backup completed in {entry['duration_seconds']:.1f}s")

        except Exception as e:
            entry["status"] = "error"
            entry["error"] = str(e)
            entry["duration_seconds"] = (datetime.now() - start_time).total_seconds()
            logger.error(f"Scheduled backup failed: {e}")

        finally:
            self.backup_service.log_backup(entry)

    def _load_config(self) -> Dict:
        """Load schedule config from persistent file"""
        if os.path.exists(SCHEDULE_CONFIG_FILE):
            with open(SCHEDULE_CONFIG_FILE, 'r') as f:
                return json.load(f)
        return {
            "enabled": False,
            "cron": "0 2 * * *",
            "provider": None,
            "remote_path": "rag-enterprise-backups",
            "retention": 5
        }

    def _save_config(self):
        """Save schedule config to persistent file"""
        os.makedirs(os.path.dirname(SCHEDULE_CONFIG_FILE), exist_ok=True)
        with open(SCHEDULE_CONFIG_FILE, 'w') as f:
            json.dump(self._config, f, indent=2)
