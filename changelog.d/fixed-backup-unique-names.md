- **Two backups started in the same second no longer overwrite each
  other.** Archive and work-dir names had one-second granularity, so a
  double-clicked "Run Backup Now" (or a manual run landing on the cron
  tick) silently replaced the first archive with the second. Names now
  carry a short random suffix after the timestamp prefix the listing
  sorts on.
