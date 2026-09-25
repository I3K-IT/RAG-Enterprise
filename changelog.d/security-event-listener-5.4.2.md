- **`event-listener` 5.4.2, for RUSTSEC-2026-0221.** 5.4.1 let a
  `!Send` tag cross threads through its stack-allocated listener: a data
  race in safe code. It reaches this server through sqlx, which uses
  neither tags nor that listener, so nothing here could trigger it; the
  bump clears the warning `cargo audit` raised. It also drops
  `concurrent-queue` from the lockfile.
