-- Copilot token / premium-request usage summed over the job's agent runs.
ALTER TABLE jobs ADD COLUMN agent_usage jsonb;
