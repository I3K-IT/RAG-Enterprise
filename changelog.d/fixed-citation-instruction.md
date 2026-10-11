- **Citations name the file, not a placeholder.** The default instructions
  said "Cite sources as [Filename]", and models copied it literally:
  `[Filename]`, `[ibidem]`, a code name instead of a file — 27 wrong
  citations out of 472 answers with qwen3-14b, 99 with a 4B model. The
  instruction now says to cite the file name shown in square brackets
  above each chunk, copied exactly, and never a placeholder or a name that
  is not among them.
