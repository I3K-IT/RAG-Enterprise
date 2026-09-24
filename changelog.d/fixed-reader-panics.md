- **A file that crashes its reader is refused cleanly.** When a
  document reader panicked on an upload — calamine, the spreadsheet
  library, does on some `.xlsb` files — the upload answered 500 "parse
  task panicked". It now gets the usual 422 saying the file could not be
  read, and `--bench` no longer crashes on such a file.
