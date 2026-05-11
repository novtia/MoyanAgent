-- atelier schema v4: per-session LLM sampling parameters (JSON object or NULL)
ALTER TABLE sessions ADD COLUMN llm_params TEXT;
