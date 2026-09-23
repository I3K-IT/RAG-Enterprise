- **Backups can now prune old archives.** New `BACKUP__RETAIN_LAST`:
  after each successful backup, archives beyond that many newest are
  removed, oldest first. `0` (the default) disables pruning and keeps
  the historical accumulate-forever behaviour — daily backups no
  longer fill the disk unnoticed. Pruning only ever touches `backup_*`
  names, never the archive the run has just written, and only warns on
  failure, never failing the backup itself.
