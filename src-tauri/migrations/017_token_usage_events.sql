-- atelier schema v17: structured token usage logging
CREATE TABLE IF NOT EXISTS token_usage_events (
  id                TEXT PRIMARY KEY,
  created_at        INTEGER NOT NULL,
  event_kind        TEXT NOT NULL,
  session_id        TEXT,
  correlation_id    TEXT,
  message_id        TEXT,
  agent_id          TEXT,
  agent_type        TEXT,
  model             TEXT,
  provider          TEXT,
  turn_index        INTEGER,
  tool_name         TEXT,
  prompt_tokens     INTEGER,
  completion_tokens INTEGER,
  total_tokens      INTEGER,
  output_chars      INTEGER,
  output_bytes      INTEGER,
  is_error          INTEGER NOT NULL DEFAULT 0,
  metadata_json     TEXT
);
CREATE INDEX IF NOT EXISTS idx_token_events_session ON token_usage_events(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_token_events_model ON token_usage_events(model, created_at);
CREATE INDEX IF NOT EXISTS idx_token_events_kind ON token_usage_events(event_kind, created_at);
