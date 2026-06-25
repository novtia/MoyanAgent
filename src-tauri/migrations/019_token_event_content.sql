-- atelier schema v19: full content capture for token usage events (debug logging)
ALTER TABLE token_usage_events ADD COLUMN content_json TEXT;
