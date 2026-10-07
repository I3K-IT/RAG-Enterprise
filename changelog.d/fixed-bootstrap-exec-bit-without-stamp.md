- **A binary that lost its executable bit is repaired with or without its
  stamp.** The permission check added for stamped components now also runs
  when the digest is verified the slow way, so an install whose stamp and
  mode were both lost (an unzip of the binary alone) is made runnable on the
  first start instead of failing to launch until the next one. The
  "downloading again" warning no longer blames the sha256 when the reason
  was the permission.
