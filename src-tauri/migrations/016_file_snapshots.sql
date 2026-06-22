-- Per-message snapshots of agent file mutations (create / update / delete).
--
-- Unlike role_state_snapshots (one row per message holding the FULL board),
-- a single assistant message can touch MANY files, so this table stores one
-- row per file operation. Each row captures the file's state BEFORE the
-- mutation so deleting / regenerating a message can roll the workspace back:
--
--   - op = 'create'  → before_existed = 0; rollback = delete the file
--   - op = 'update'  → before_existed = 1, before_content = old text;
--                      rollback = write old text back
--   - op = 'delete'  → before_existed = 1, before_content = old text;
--                      rollback = recreate the file
--
-- `restorable` is 0 when the pre-image was binary or too large to capture;
-- such rows cannot restore content (but 'create' rollbacks still delete).
CREATE TABLE IF NOT EXISTS file_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    path TEXT NOT NULL,
    op TEXT NOT NULL,
    before_existed INTEGER NOT NULL,
    before_content TEXT,
    restorable INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_file_snapshots_session
    ON file_snapshots (session_id, id);

CREATE INDEX IF NOT EXISTS idx_file_snapshots_message
    ON file_snapshots (message_id);
