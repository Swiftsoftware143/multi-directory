-- 070_full_schema_reconciliation: recreate missing base tables in dependency order.
-- The original Docker image shipped an empty migrations/ dir, so base CREATE TABLE
-- statements either never ran or silently failed (parent tables missing at apply time).
-- Re-applying them explicitly restores the canonical schema.

-- ── 1. visitor_accounts (prereq for event_rsvps, visitor_favorites, polls) ──
CREATE TABLE IF NOT EXISTS visitor_accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    name TEXT,
    phone TEXT,
    directory_id UUID REFERENCES directories(id) ON DELETE SET NULL,
    is_active BOOLEAN DEFAULT true,
    last_login_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_visitor_accounts_email ON visitor_accounts(email);
CREATE INDEX IF NOT EXISTS idx_visitor_accounts_directory ON visitor_accounts(directory_id);

-- ── 2. community_events + event_rsvps (events feed) ──
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
    category TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    max_attendees INTEGER,
    created_by UUID REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_community_events_directory ON community_events(directory_id);
CREATE INDEX IF NOT EXISTS idx_community_events_date ON community_events(event_date);
CREATE INDEX IF NOT EXISTS idx_community_events_status ON community_events(status);

CREATE TABLE IF NOT EXISTS event_rsvps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL REFERENCES community_events(id) ON DELETE CASCADE,
    visitor_account_id UUID NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'going',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(event_id, visitor_account_id)
);
CREATE INDEX IF NOT EXISTS idx_event_rsvps_event ON event_rsvps(event_id);
CREATE INDEX IF NOT EXISTS idx_event_rsvps_visitor ON event_rsvps(visitor_account_id);

-- ── 3. directory_events (legacy event feed) ──
CREATE TABLE IF NOT EXISTS directory_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    event_type TEXT,
    payload JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_directory_events_directory ON directory_events(directory_id);

-- ── 4. reviews (activity feed) ──
CREATE TABLE IF NOT EXISTS reviews (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    reviewer_name TEXT,
    reviewer_email TEXT,
    rating INTEGER,
    content TEXT,
    status TEXT DEFAULT 'pending',
    featured BOOLEAN DEFAULT false,
    source TEXT DEFAULT 'direct',
    source_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_reviews_business ON reviews(business_id);
CREATE INDEX IF NOT EXISTS idx_reviews_status ON reviews(status);
CREATE INDEX IF NOT EXISTS idx_reviews_directory ON reviews(directory_id);

-- ── 5. visitor_favorites ──
CREATE TABLE IF NOT EXISTS visitor_favorites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    visitor_account_id UUID NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(visitor_account_id, business_id)
);
CREATE INDEX IF NOT EXISTS idx_visitor_favorites_visitor ON visitor_favorites(visitor_account_id);
CREATE INDEX IF NOT EXISTS idx_visitor_favorites_directory ON visitor_favorites(directory_id);

-- ── 6. polls + poll_votes ──
CREATE TABLE IF NOT EXISTS polls (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    options TEXT[] NOT NULL DEFAULT '{}',
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'active',
    starts_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ends_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_polls_directory ON polls(directory_id);

CREATE TABLE IF NOT EXISTS poll_votes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    poll_id UUID NOT NULL REFERENCES polls(id) ON DELETE CASCADE,
    visitor_account_id UUID NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    option_index INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(poll_id, visitor_account_id)
);

-- ── 7. directory_categories (categories filter) ──
CREATE TABLE IF NOT EXISTS directory_categories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) NOT NULL,
    icon TEXT,
    display_order INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_directory_categories_directory ON directory_categories(directory_id);

-- ── 8. category_requests (city/category request feature) ──
CREATE TABLE IF NOT EXISTS category_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    name VARCHAR(255),
    status TEXT DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── 9. city_requests ──
CREATE TABLE IF NOT EXISTS city_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    city_name VARCHAR(255),
    state VARCHAR(100),
    status TEXT DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── 10. call_logs + twilio_numbers ──
CREATE TABLE IF NOT EXISTS call_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID REFERENCES businesses(id) ON DELETE CASCADE,
    from_number VARCHAR(50),
    to_number VARCHAR(50),
    status TEXT,
    duration_seconds INTEGER,
    recording_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS twilio_numbers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    phone_number VARCHAR(50),
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
