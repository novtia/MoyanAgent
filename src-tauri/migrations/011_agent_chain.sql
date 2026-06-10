-- Per-session ordered agent flow chain (JSON array of agent_type strings).
-- NULL / empty means: fall back to the single `agent_type` generation flow.
ALTER TABLE sessions ADD COLUMN agent_chain TEXT;

-- User-defined sub-agents, saved globally so they can be reused across
-- sessions and arranged into agent flow chains.
CREATE TABLE IF NOT EXISTS custom_agents (
    agent_type TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    when_to_use TEXT NOT NULL DEFAULT '',
    system_prompt TEXT NOT NULL DEFAULT '',
    model TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
