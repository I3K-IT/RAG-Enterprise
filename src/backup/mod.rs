//! Backup and restore: a tar.gz holding the SQLite database and a Qdrant
//! snapshot, taken together so the pair is consistent, and put back together
//! by `service::restore_backup`.
//!
//! Backups are local files under `BACKUP__DIR`. Nothing is uploaded anywhere.

pub mod scheduler;
pub mod service;
