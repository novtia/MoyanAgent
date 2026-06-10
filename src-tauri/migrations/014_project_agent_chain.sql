-- Project-level agent flow chain.
--
-- Sessions belonging to a project share a single, project-scoped agent flow
-- (one record per project) instead of each session storing its own chain.
-- Stored as a JSON array of agent_type strings; NULL means single-agent runs.
ALTER TABLE projects ADD COLUMN agent_chain TEXT;
