//! Backup service: tar+zstd of SQLite DB + Qdrant snapshot + optional rclone upload.
//!
//! Mirrors Python backup_service (PIANO §4 Community feature list).
//! TODO Fase 1: implement backup_service, scheduler, rclone trigger, qdrant_snapshot.

pub mod scheduler;
pub mod service;
