//! Backup service: tar+zstd of SQLite DB + Qdrant snapshot + optional rclone upload.
//!
//!
//! The SQLite dump and the Qdrant snapshot are taken together so that a
//! restore brings back a consistent pair.

pub mod scheduler;
pub mod service;
