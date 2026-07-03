-- Role state snapshots are keyed by scope_id: project_id for project sessions,
-- session_id for standalone sessions. session_id column records which session
-- produced each snapshot (for debugging); rollback still uses message_id.

ALTER TABLE role_state_snapshots ADD COLUMN scope_id TEXT;

UPDATE role_state_snapshots
SET scope_id = COALESCE(
    (SELECT project_id FROM sessions WHERE sessions.id = role_state_snapshots.session_id),
    session_id
);

-- SQLite cannot ALTER COLUMN to NOT NULL; enforce via application layer.
CREATE INDEX IF NOT EXISTS idx_role_state_scope
    ON role_state_snapshots (scope_id, id);
