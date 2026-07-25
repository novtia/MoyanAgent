-- atelier schema v26: prompt-cache token columns on usage events
ALTER TABLE token_usage_events ADD COLUMN cache_read_tokens INTEGER;
ALTER TABLE token_usage_events ADD COLUMN cache_write_tokens INTEGER;
