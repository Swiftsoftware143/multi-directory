-- BL29: Pricing Engine — configurable service pricing per directory/network

CREATE TABLE IF NOT EXISTS service_prices (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    network_id UUID REFERENCES networks(id) ON DELETE CASCADE,
    service_key VARCHAR(100) NOT NULL,
    price_monthly NUMERIC(10,2),
    price_yearly NUMERIC(10,2),
    price_one_time NUMERIC(10,2),
    currency VARCHAR(3) DEFAULT 'USD',
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    CONSTRAINT unique_service_per_scope UNIQUE (directory_id, network_id, service_key),
    CONSTRAINT check_scope CHECK (
        (directory_id IS NOT NULL AND network_id IS NULL) OR
        (directory_id IS NULL AND network_id IS NOT NULL) OR
        (directory_id IS NULL AND network_id IS NULL)
    )
);

CREATE TABLE IF NOT EXISTS price_bundles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    network_id UUID REFERENCES networks(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) UNIQUE NOT NULL,
    description TEXT,
    price_monthly NUMERIC(10,2),
    price_yearly NUMERIC(10,2),
    is_active BOOLEAN DEFAULT true,
    sort_order INTEGER DEFAULT 0,
    is_featured BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS bundle_services (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id UUID NOT NULL REFERENCES price_bundles(id) ON DELETE CASCADE,
    service_key VARCHAR(100) NOT NULL,
    UNIQUE(bundle_id, service_key)
);

CREATE TABLE IF NOT EXISTS grandfathered_pricing (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    service_key VARCHAR(100) NOT NULL,
    price_monthly NUMERIC(10,2),
    price_yearly NUMERIC(10,2),
    price_one_time NUMERIC(10,2),
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(business_id, service_key)
);

INSERT INTO service_prices (directory_id, network_id, service_key, price_monthly, price_yearly, price_one_time, is_active)
VALUES
    (NULL, NULL, 'claim_listing', 0, 0, 0, true),
    (NULL, NULL, 'deals', 29.00, 290.00, NULL, true),
    (NULL, NULL, 'blogging', 49.00, 490.00, 19.00, true),
    (NULL, NULL, 'community_posts', 19.00, 190.00, 9.00, true),
    (NULL, NULL, 'b2b_marketplace', 99.00, 990.00, NULL, true),
    (NULL, NULL, 'premium_listing', 19.00, 190.00, NULL, true)
ON CONFLICT (directory_id, network_id, service_key) DO NOTHING;