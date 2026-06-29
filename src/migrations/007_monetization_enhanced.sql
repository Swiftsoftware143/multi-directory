-- Monetization enhancements: directory tiers + sponsored listings

CREATE TABLE IF NOT EXISTS directory_tiers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    tier_slug TEXT NOT NULL,
    tier_name TEXT NOT NULL DEFAULT 'Free',
    is_active BOOLEAN DEFAULT true,
    started_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    stripe_subscription_id TEXT,
    stripe_customer_id TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(directory_id)
);

CREATE TABLE IF NOT EXISTS sponsored_listings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    slot_position INTEGER NOT NULL DEFAULT 1,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    is_active BOOLEAN DEFAULT true,
    price_paid DECIMAL(10,2) DEFAULT 0,
    currency TEXT DEFAULT 'USD',
    stripe_payment_intent_id TEXT,
    featured BOOLEAN DEFAULT false,
    badge_text TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_directory_tiers_directory ON directory_tiers(directory_id);
CREATE INDEX IF NOT EXISTS idx_sponsored_listings_directory ON sponsored_listings(directory_id);
CREATE INDEX IF NOT EXISTS idx_sponsored_listings_business ON sponsored_listings(business_id);
CREATE INDEX IF NOT EXISTS idx_sponsored_listings_active ON sponsored_listings(directory_id, is_active, end_date);
