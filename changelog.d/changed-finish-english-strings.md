- **The application's last Italian text is now English.** In the web
  UI: the admin panel's Qdrant and SQLite tabs, the chat's error and
  timeout messages, the source list and the ingestion banner — and the
  dates in that panel now follow the browser's locale instead of being
  forced to Italian. In the server: the first-install banner that
  prints the generated admin password, the `embedding_device` value
  `/api/info` reports when the embedding lock is poisoned, the warning
  logged when a CUDA embedding batch runs out of memory, and the doc
  comments and log lines left over from earlier passes. Italian that is
  there on purpose stays: the heading keywords the chunker matches in
  Italian documents (`articolo`, `capitolo`, …), the multilingual
  example inside the answer prompt, and Italian test fixtures.
