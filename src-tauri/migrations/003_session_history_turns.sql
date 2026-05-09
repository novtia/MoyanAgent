-- atelier schema v3
ALTER TABLE sessions ADD COLUMN history_turns INTEGER NOT NULL DEFAULT 10;

UPDATE sessions
SET history_turns = COALESCE(
  CAST((SELECT value FROM settings WHERE key = 'history_turns') AS INTEGER),
  10
)
WHERE history_turns = 10;
