-- Phase 4: Data Company features
-- Google Places autofill cache, business verification, data enrichment

CREATE TABLE IF NOT EXISTS google_places_cache (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query TEXT NOT NULL,
    place_id TEXT,
    name TEXT,
    formatted_address TEXT,
    phone TEXT,
    website TEXT,
    latitude DOUBLE PRECISION,
    longitude DOUBLE PRECISION,
    rating DOUBLE PRECISION,
    user_ratings_total INTEGER,
    types TEXT[],
    photos TEXT[],
    opening_hours JSONB,
    place_details JSONB,
    cached_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ DEFAULT (NOW() + INTERVAL '7 days')
);

CREATE INDEX idx_google_places_cache_query ON google_places_cache(query);
CREATE INDEX idx_google_places_cache_place_id ON google_places_cache(place_id);

CREATE TABLE IF NOT EXISTS business_verifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID REFERENCES directories(id),
    method TEXT NOT NULL DEFAULT 'manual',
    status TEXT NOT NULL DEFAULT 'pending',
    verified_by UUID REFERENCES users(id),
    verified_at TIMESTAMPTZ,
    verification_doc_url TEXT,
    notes TEXT,
    expires_at TIMESTAMPTZ,
    verified_data JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_business_verifications_business ON business_verifications(business_id);
CREATE INDEX idx_business_verifications_status ON business_verifications(status);

CREATE TABLE IF NOT EXISTS data_enrichment_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID REFERENCES businesses(id) ON DELETE SET NULL,
    directory_id UUID REFERENCES directories(id),
    source TEXT NOT NULL,
    enrichment_type TEXT NOT NULL,
    data_before JSONB,
    data_after JSONB,
    confidence DOUBLE PRECISION DEFAULT 1.0,
    status TEXT NOT NULL DEFAULT 'completed',
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_data_enrichment_logs_business ON data_enrichment_logs(business_id);
