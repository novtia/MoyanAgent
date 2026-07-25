-- Bind pending Edit review hunks to the chat turn that produced them so
-- delete_message can roll files back even when file_snapshots are missing.
ALTER TABLE pending_diffs ADD COLUMN request_message_id TEXT;
ALTER TABLE pending_diffs ADD COLUMN message_id TEXT;

CREATE INDEX IF NOT EXISTS idx_pending_diffs_request_message
    ON pending_diffs (session_id, request_message_id);

CREATE INDEX IF NOT EXISTS idx_pending_diffs_message
    ON pending_diffs (session_id, message_id);
