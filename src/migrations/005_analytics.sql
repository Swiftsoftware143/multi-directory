-- Analytics events table
CREATE TABLE IF NOT EXISTS analytics_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    entity_type TEXT,
    entity_id UUID,
    directory_id UUID REFERENCES directories(id) ON DELETE SET NULL,
    metadata JSONB,
    ip_address TEXT,
    user_agent TEXT,
    referrer TEXT,
    session_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_analytics_event_type ON analytics_events(event_type);
CREATE INDEX IF NOT EXISTS idx_analytics_created_at ON analytics_events(created_at);
CREATE INDEX IF NOT EXISTS idx_analytics_directory ON analytics_events(directory_id);
