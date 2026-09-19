- **The `top_k` request field is honored instead of silently ignored.**
  Every query carried `top_k` (the frontend sends `5`) while the backend
  always retrieved the hardcoded 15 — a contract lie that also spent
  roughly three times the intended context on every question. Absent
  still means the 15 default; present is clamped to `1..=50`, so no
  working client breaks and no value fans out to Qdrant unbounded.
