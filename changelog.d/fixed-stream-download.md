- **Downloads stream instead of buffering the whole file in RAM.**
  `GET /api/documents/{id}/download` loaded the entire original (up to
  the 1024 MB cap) before responding — the same peak the upload path
  just stopped paying. The file now streams with a `Content-Length`
  header; headers and 404 behavior are unchanged. A read failing
  mid-stream truncates rather than 404s, inherent to streaming.
