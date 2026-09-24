- **The upload dialog no longer offers `.pptx` and no longer hides
  `.md`/`.csv`.** The file picker's allow-list had drifted from
  `SUPPORTED_EXTENSIONS`: `.pptx` was offered but refused after
  selection, while `.md` and `.csv` parsed fine but were undiscoverable.
  One-line sync plus the rebuilt bundle, per repo convention.
