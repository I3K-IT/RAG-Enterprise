-- Settings an administrator changes at runtime, one row per key. A key with
-- no row takes its default from the code (see db::settings), so this table
-- starts empty and an install that never changes anything never writes it.

CREATE TABLE IF NOT EXISTS settings (
    key         TEXT    PRIMARY KEY,
    value       TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,           -- ISO UTC
    updated_by  TEXT                        -- username of the admin who set it
);
