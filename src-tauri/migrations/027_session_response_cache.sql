-- atelier schema v27: Volcengine Ark Responses API session cache chain
ALTER TABLE sessions ADD COLUMN last_response_id TEXT;
ALTER TABLE sessions ADD COLUMN cache_thinking_key TEXT;
