- **`EULLM__EXTRA_ARGS` no longer stops a service that does not start
  eullm.** With `DATA__MANAGE_SUBPROCESSES=false` the value is never used,
  yet a duplicated option or an unclosed quote in it refused startup. It is
  now checked only when RAG starts eullm itself, and otherwise reported as
  ignored.
