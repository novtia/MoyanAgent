-- Rebuild `file_snapshots` so a row can exist before its assistant message does.
--
-- Snapshots used to be buffered in memory and flushed only when a generation
-- finalized, which meant a cancelled or crashed turn left files on disk with no
-- rollback record. Rows are now inserted at write time, so `message_id` has to
-- be nullable and `request_message_id` carries the originating user turn until
-- the assistant message binds them. `rollback_error` keeps a failed restore
-- visible instead of silently dropping the pre-image.

CREATE TABLE file_snapshots_new (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id         TEXT NOT NULL,
  message_id         TEXT,
  request_message_id TEXT,
  path               TEXT NOT NULL,
  op                 TEXT NOT NULL,
  before_existed     INTEGER NOT NULL,
  before_content     TEXT,
  restorable         INTEGER NOT NULL,
  created_at         INTEGER NOT NULL,
  before_encoding    TEXT,
  before_had_bom     INTEGER NOT NULL DEFAULT 0,
  rollback_error     TEXT
);

INSERT INTO file_snapshots_new (
  id, session_id, message_id, request_message_id, path, op,
  before_existed, before_content, restorable, created_at,
  before_encoding, before_had_bom, rollback_error
)
SELECT
  id, session_id, message_id, NULL, path, op,
  before_existed, before_content, restorable, created_at,
  before_encoding, before_had_bom, NULL
FROM file_snapshots;

DROP TABLE file_snapshots;
ALTER TABLE file_snapshots_new RENAME TO file_snapshots;

CREATE INDEX IF NOT EXISTS idx_file_snapshots_session ON file_snapshots(session_id, id);
CREATE INDEX IF NOT EXISTS idx_file_snapshots_message ON file_snapshots(message_id);
CREATE INDEX IF NOT EXISTS idx_file_snapshots_request ON file_snapshots(session_id, request_message_id);
