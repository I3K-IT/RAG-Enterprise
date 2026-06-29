//! Periodic backup scheduler (daily at 02:00 UTC via tokio-cron-scheduler).

use anyhow::Result;
use sqlx::SqlitePool;
use tokio_cron_scheduler::{Job, JobScheduler};

/// Start a background scheduler that runs a backup daily at 02:00 UTC.
/// Arguments are cloned into the job closure.
pub async fn start(
    db: SqlitePool,
    db_path: String,
    qdrant_url: String,
    qdrant_collection: String,
    backup_dir: String,
) -> Result<()> {
    let scheduler = JobScheduler::new().await?;

    let job = Job::new_async("0 0 2 * * *", move |_uuid, _lock| {
        let db = db.clone();
        let db_path = db_path.clone();
        let qdrant_url = qdrant_url.clone();
        let qdrant_collection = qdrant_collection.clone();
        let backup_dir = backup_dir.clone();
        Box::pin(async move {
            tracing::info!("scheduled backup starting");
            match super::service::create_backup(
                &db,
                &db_path,
                &qdrant_url,
                &qdrant_collection,
                &backup_dir,
            )
            .await
            {
                Ok(path) => tracing::info!(archive = %path.display(), "scheduled backup ok"),
                Err(e) => tracing::error!(error = %e, "scheduled backup failed"),
            }
        })
    })?;

    scheduler.add(job).await?;
    scheduler.start().await?;

    tracing::info!("backup scheduler started (daily 02:00 UTC)");
    Ok(())
}
