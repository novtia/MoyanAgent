-- Per custom-agent allowed tool list (JSON array of tool names).
-- NULL / empty array means: full tool access ("*"), preserving the previous
-- behaviour for rows created before this migration.
ALTER TABLE custom_agents ADD COLUMN tools TEXT;
