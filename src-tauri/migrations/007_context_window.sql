-- atelier schema v7: context window (tokens) on catalog models and sessions
PRAGMA foreign_keys = ON;

-- Builtin / catalog models: max context window size (tokens). NULL = unknown / unspecified.
ALTER TABLE llm_sdk_model ADD COLUMN context_window INTEGER;
ALTER TABLE llm_supplier_model ADD COLUMN context_window INTEGER;

-- Per-session snapshot of limit + cumulative usage (tokens). NULL limit defers to model/catalog.
ALTER TABLE sessions ADD COLUMN context_window INTEGER;
ALTER TABLE sessions ADD COLUMN context_window_used INTEGER NOT NULL DEFAULT 0;
