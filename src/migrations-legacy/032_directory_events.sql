-- Migration 032: Community events with RSVP

CREATE TABLE IF NOT EXISTS community_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID REFERENCES businesses(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    description TEXT,
    event_date TIMESTAMPTZ NOT NULL,
    end_date TIMESTAMPTZ,
    location TEXT,
    address TEXT,
    image_url TEXT,
    category TEXT, -- 'community', 'food', 'music', 'sports', 'business', 'workshop', 'other'
    status TEXT NOT NULL DEFAULT 'active', -- 'active', 'cancelled', 'completed'
    max_attendees INTEGER,
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS event_rsvps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES community_events(id) ON DELETE CASCADE,
    visitor_account_id UUID NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'going', -- 'going', 'maybe', 'not-going'
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(event_id, visitor_account_id)
);

CREATE INDEX IF NOT EXISTS idx_community_events_directory ON community_events(directory_id);
CREATE INDEX IF NOT EXISTS idx_community_events_date ON community_events(event_date);
CREATE INDEX IF NOT EXISTS idx_community_events_status ON community_events(status);
CREATE INDEX IF NOT EXISTS idx_event_rsvps_event ON event_rsvps(event_id);
CREATE INDEX IF NOT EXISTS idx_event_rsvps_visitor ON event_rsvps(visitor_account_id);

INSERT INTO _migrations (filename) VALUES ('032_directory_events.sql');
