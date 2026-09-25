- **`spin` 0.9.9 replaces 0.9.8, which is yanked.** Its author yanked
  every earlier 0.9 release when 0.9.9 came out in July, without saying
  why, and no advisory names it. It reaches this server through axum's
  multipart parser and sqlx's SQLite driver. A patch bump in
  `Cargo.lock`; no code changed.
