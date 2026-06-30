-- Conversazioni persistenti in SQLite.
-- Ogni riga = una "chat" dell'utente con titolo e timestamp.
-- chat_messages acquisisce conversation_id per linkare i messaggi.

CREATE TABLE IF NOT EXISTS conversations (
    id          TEXT    PRIMARY KEY,        -- UUID4
    user_id     INTEGER NOT NULL,
    title       TEXT    NOT NULL DEFAULT 'New Conversation',
    created_at  TEXT    NOT NULL,           -- ISO UTC
    updated_at  TEXT    NOT NULL,           -- aggiornato ad ogni messaggio
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE INDEX IF NOT EXISTS idx_conv_user
    ON conversations (user_id, updated_at DESC);

-- Aggiunge la colonna a chat_messages (nullable: i messaggi pre-migrazione hanno NULL)
ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT
    REFERENCES conversations(id);

CREATE INDEX IF NOT EXISTS idx_chat_conv
    ON chat_messages (conversation_id, id ASC);
