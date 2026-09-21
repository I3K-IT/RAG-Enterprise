- **Regression coverage for four untested pure helpers.**
  `expand_tilde` (data-dir resolution), `fmt_bytes`/`fmt_eta`
  (bootstrap progress output) and `is_bad_request` (the 400-vs-500
  split on restore failures) were deterministic and load-bearing but
  had zero tests. Covered now, boundaries included. `l2_normalize`
  was deliberately left out: it is dead code, and testing it would
  cement it instead of questioning it.
