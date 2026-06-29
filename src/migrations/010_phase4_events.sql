-- Phase 4: Directory events table (for n8n automation)

CREATE TABLE IF NOT EXISTS directory_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id UUID,
    directory_id UUID REFERENCES directories(id),
    tenant_id UUID REFERENCES tenants(id),
    actor_id UUID REFERENCES users(id),
    data JSONB DEFAULT '{}',
    metadata JSONB DEFAULT '{}',
    processed BOOLEAN DEFAULT false,
    n8n_webhook_sent BOOLEAN DEFAULT false,
    n8n_webhook_failed BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_directory_events_type ON directory_events(event_type);
CREATE INDEX idx_directory_events_entity ON directory_events(entity_type, entity_id);
CREATE INDEX idx_directory_events_directory ON directory_events(directory_id);
CREATE INDEX idx_directory_events_created ON directory_events(created_at);
CREATE INDEX idx_directory_events_unprocessed ON directory_events(processed, created_at) WHERE processed = false;
