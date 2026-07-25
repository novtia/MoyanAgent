-- Pending Edit review hunks for the reader Keep/Undo UI.
--
-- Edit tools write to disk immediately; each successful Edit also inserts a
-- row here so the frontend can restore confirmation state after closing a
-- tab or restarting. Accept deletes the row; reject restores text_before
-- and drops this row plus every later hunk on the same path.
CREATE TABLE IF NOT EXISTS pending_diffs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    path TEXT NOT NULL,
    before_snippet TEXT NOT NULL,
    after_snippet TEXT NOT NULL,
    text_before TEXT NOT NULL,
    text_after TEXT NOT NULL,
    encoding TEXT,
    had_bom INTEGER NOT NULL DEFAULT 0,
    seq INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_diffs_session_path
    ON pending_diffs (session_id, path, seq);
