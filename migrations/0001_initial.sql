-- Initial schema: rag_users.db tables (MAPPA §4)
-- No PRAGMA here — mirrors Python database.py (no WAL on users DB)

CREATE TABLE IF NOT EXISTS users (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    username     TEXT    UNIQUE NOT NULL,
    email        TEXT    UNIQUE NOT NULL,
    password_hash TEXT   NOT NULL,
    role         TEXT    NOT NULL,          -- admin | super_user | user
    created_at   TEXT    NOT NULL,          -- ISO UTC
    last_login   TEXT,
    is_active    INTEGER NOT NULL DEFAULT 1 -- soft delete
);

CREATE TABLE IF NOT EXISTS chat_messages (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL,
    role      TEXT    NOT NULL,             -- user | assistant
    content   TEXT    NOT NULL,
    sources   TEXT,                         -- JSON serialised
    timestamp TEXT    NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE INDEX IF NOT EXISTS idx_chat_user_id
    ON chat_messages (user_id, timestamp DESC);

CREATE TABLE IF NOT EXISTS documents (
    id          TEXT    PRIMARY KEY,        -- UUID4
    filename    TEXT    NOT NULL,
    upload_date TEXT    NOT NULL,           -- ISO UTC
    page_count  INTEGER,
    doc_type    TEXT    NOT NULL DEFAULT 'document',
    chunk_count INTEGER NOT NULL DEFAULT 0,
    is_deleted  INTEGER NOT NULL DEFAULT 0
);
