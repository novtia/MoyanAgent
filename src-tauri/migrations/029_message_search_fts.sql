-- Message/session FTS5 trigram index for cross-session search + in-chat find.
-- Applied when upgrading from squashed baseline schema_version=28.
--
-- NOTE: runtime migration is driven by `data::db::ensure_message_search_schema`
-- (idempotent column/table checks). This file documents the target DDL.

ALTER TABLE messages ADD COLUMN searchable_text TEXT NOT NULL DEFAULT '';

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  searchable_text,
  content='messages',
  content_rowid='rowid',
  tokenize='trigram'
);

CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
  title,
  content='sessions',
  content_rowid='rowid',
  tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, searchable_text)
    VALUES('delete', old.rowid, old.searchable_text);
END;

CREATE TRIGGER IF NOT EXISTS sessions_fts_ad AFTER DELETE ON sessions BEGIN
  INSERT INTO sessions_fts(sessions_fts, rowid, title)
    VALUES('delete', old.rowid, old.title);
END;
