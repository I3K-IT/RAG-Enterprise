- **BUILD.md no longer shows a stale database default and documents
  `DATA__DIR` for source builds.** The runtime table listed
  `DATABASE__URL=sqlite://rag_users.db`, but the default has long been
  `{DATA__DIR}/db/rag_users.db` — and nothing told a `cargo run`
  developer that the data root defaults to `target/debug/`. Both fixed
  where a source builder actually reads them.
