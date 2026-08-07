CREATE UNIQUE INDEX IF NOT EXISTS idx_events_source ON community_events (directory_id, source_provider_id, source_event_id)
    WHERE source_provider_id IS NOT NULL AND source_event_id IS NOT NULL
