- **The chat history no longer stores a copy of every source's text.**
  Each answer was saved together with the full text of every chunk it was
  built from — up to 15 of them, 10–15 KB per question, kept for good —
  although nothing ever displayed it: the web UI shows a source's file,
  pages and score. Stored sources now keep only what locates the passage
  (document, chunk, byte range, pages), so each question takes about a
  quarter of the space. The live answer still carries the chunk text.
  Messages already in the history are left as they are; for new ones,
  `GET /api/chat/history` returns sources without `text`.
