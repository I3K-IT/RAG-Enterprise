- **Startup now refuses a misconfigured eullm context sizing.**
  `EULLM__NUM_CTX * EULLM__BATCH_SIZE` becomes `--ctx-size`, but neither
  side was validated: `0` started eullm with no context or no slot, and
  an absurd pair overflowed `u32` — panicking in debug, wrapping in
  release into a garbage context a RAG prompt will not fit in. Both are
  rejected fail-fast in `Settings::load`, following the existing
  `validate_auth`/`validate_storage` pattern.
