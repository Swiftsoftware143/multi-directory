ALTER TABLE community_events ADD COLUMN IF NOT EXISTS source_provider_id UUID REFERENCES event_providers(id);
ALTER TABLE community_events ADD COLUMN IF NOT EXISTS source_event_id TEXT
