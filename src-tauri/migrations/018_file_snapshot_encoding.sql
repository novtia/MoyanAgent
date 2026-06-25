ALTER TABLE file_snapshots ADD COLUMN before_encoding TEXT;
ALTER TABLE file_snapshots ADD COLUMN before_had_bom INTEGER NOT NULL DEFAULT 0;
