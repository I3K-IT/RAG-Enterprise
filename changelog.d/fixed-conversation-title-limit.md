- **Conversation titles are capped at 200 characters.** `PUT
  /api/conversations/{id}` only rejected empty titles, so anything up
  to the 2 MB JSON body limit was stored verbatim and returned on
  every list call — while the UI only ever shows ~50 characters. Now
  enforced up front via a tested `validate_title`, mirroring
  `validate_query` (characters, not bytes).
