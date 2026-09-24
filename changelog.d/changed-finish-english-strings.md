- **The last Italian strings are now English.** Doc comments, section
  headers and log lines left over from earlier passes, plus three
  things an operator can actually see: the first-install banner that
  prints the generated admin password, the `embedding_device` value
  `/api/info` reports when the embedding lock is poisoned, and the
  warning logged when a CUDA embedding batch runs out of memory and is
  retried on CPU. Italian that is there on purpose stays: the heading
  keywords the chunker matches in Italian documents (`articolo`,
  `capitolo`, …), the multilingual example inside the answer prompt,
  and Italian test fixtures.
