-- atelier schema v5: agent runtime metadata
--   sessions.agent_type   which agent definition drives this session
--   messages.events_json  JSON-serialized AgentEvent[] streamed during the turn
ALTER TABLE sessions ADD COLUMN agent_type TEXT NOT NULL DEFAULT 'general-purpose';
ALTER TABLE messages ADD COLUMN events_json TEXT;
