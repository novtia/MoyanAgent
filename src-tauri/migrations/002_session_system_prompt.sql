-- atelier schema v2
ALTER TABLE sessions ADD COLUMN system_prompt TEXT NOT NULL DEFAULT '';

UPDATE sessions
SET system_prompt = COALESCE(
  (SELECT value FROM settings WHERE key = 'system_prompt'),
  ''
)
WHERE system_prompt = '';
