-- Per-message snapshots of the character state board.
--
-- One row per assistant message that touched the board. `state_json` holds
-- the FULL set of roles (a JSON array) as it stood after that message, so
-- deleting a message can roll the in-memory store back to the previous
-- snapshot, and opening a session can re-hydrate the latest one.
CREATE TABLE IF NOT EXISTS role_state_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    state_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_role_state_session
    ON role_state_snapshots (session_id, id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_role_state_message
    ON role_state_snapshots (message_id);
