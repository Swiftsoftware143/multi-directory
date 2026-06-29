-- Monetization tables for Multi-Directory API

CREATE TABLE IF NOT EXISTS plan_tiers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    price_monthly DECIMAL(10,2) DEFAULT 0,
    price_yearly DECIMAL(10,2) DEFAULT 0,
    max_listings INTEGER DEFAULT -1,
    max_deals INTEGER DEFAULT 0,
    max_photos INTEGER DEFAULT 5,
    has_reviews BOOLEAN DEFAULT true,
    has_analytics BOOLEAN DEFAULT false,
    has_crm BOOLEAN DEFAULT false,
    has_email BOOLEAN DEFAULT false,
    has_call_tracking BOOLEAN DEFAULT false,
    has_import_export BOOLEAN DEFAULT false,
    has_api_access BOOLEAN DEFAULT false,
    featured_listing BOOLEAN DEFAULT false,
    description TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS business_subscriptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL,
    tier_id UUID REFERENCES plan_tiers(id),
    status TEXT DEFAULT 'active',
    billing_cycle TEXT DEFAULT 'monthly',
    price_paid DECIMAL(10,2),
    currency TEXT DEFAULT 'USD',
    start_date DATE NOT NULL,
    end_date DATE,
    auto_renew BOOLEAN DEFAULT true,
    stripe_subscription_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS ad_zones (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    zone_key TEXT NOT NULL,
    width INTEGER DEFAULT 300,
    height INTEGER DEFAULT 250,
    price_monthly DECIMAL(10,2),
    directory_id UUID REFERENCES directories(id),
    status TEXT DEFAULT 'available',
    current_advertiser_id UUID,
    current_ad_url TEXT,
    current_ad_image TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
