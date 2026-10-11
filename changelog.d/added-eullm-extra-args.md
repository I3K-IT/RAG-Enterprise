- **`EULLM__EXTRA_ARGS` passes extra options to eullm.** Appended to `eullm
  run` after the options RAG sets — e.g. `--moe-cache auto`, which takes a
  large MoE model on a single GPU from about 20 to 51–56 tokens/s in our
  tests. The value is split into arguments the way a shell would, quotes
  included, but never run through a shell. An option RAG already sets
  (`--ctx-size`, `--batch-size`, `--cache-type-k/v`, `--n-cpu-moe`,
  `--embedding-model`, `--cli`) stops startup with the variable to use
  instead, e.g. "--ctx-size is already set by RAG: use EULLM__NUM_CTX". The
  full eullm command line is logged at startup.
