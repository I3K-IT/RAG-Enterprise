- **`--bench-live` reports every ingestion stage again.** Since 0.1.25
  the live report showed 0 ms for text extraction and Qdrant upsert on
  every real upload: a translation pass renamed the stage names the
  report looks up, but not the names the upload handler records them
  under, and the lookup fell back to zero without a word. Each row's
  columns no longer added up to its own total, the averages understated
  the real time, and neither stage could ever be named the bottleneck.
  Stages are now an enum, so the two sides cannot disagree again.
