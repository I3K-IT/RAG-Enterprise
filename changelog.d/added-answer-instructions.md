- **The instructions the model answers with are the admin's to change.** The
  prompt is now two parts: instructions — role, structure, citations,
  language — which an admin edits from the new *Answers* tab of the admin
  panel, and a frame the engine keeps — history, retrieved chunks,
  question. Saved in the new `settings` table (migration 0003, included in
  backups) and read for every question, so a change applies to the next one
  without a restart; blank or reset means the built-in instructions.
  `GET`/`PUT`/`DELETE /api/admin/settings/answer-instructions`, admin only,
  up to 8,000 characters. `{context}` or `{question}` written in the
  instructions stay text. Restoring a backup taken before this release
  leaves the current instructions as they are.
