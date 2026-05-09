-- atelier schema v1
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  model      TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
  id          TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role        TEXT NOT NULL,
  text        TEXT,
  params_json TEXT,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at);

CREATE TABLE IF NOT EXISTS message_images (
  id         TEXT PRIMARY KEY,
  message_id TEXT REFERENCES messages(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role       TEXT NOT NULL,
  rel_path   TEXT NOT NULL,
  thumb_path TEXT,
  mime       TEXT NOT NULL,
  width      INTEGER,
  height     INTEGER,
  bytes      INTEGER,
  ord        INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_msgimg_msg ON message_images(message_id, ord);
CREATE INDEX IF NOT EXISTS idx_msgimg_session ON message_images(session_id);
