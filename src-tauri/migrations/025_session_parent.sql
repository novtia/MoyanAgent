-- atelier schema v25: temporary subagent child sessions
-- Parent Agent tool calls spawn a hidden temp session (not listed in sidebar).
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE;
ALTER TABLE sessions ADD COLUMN is_temporary INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN spawn_task_id TEXT;

CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_temporary ON sessions(is_temporary);
