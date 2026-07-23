-- atelier schema v22: per-session model provider id
-- Records which model service (provider) a session uses, so a session fully
-- owns its model identity (provider + model) independent of the global default.
ALTER TABLE sessions ADD COLUMN provider_id TEXT;
