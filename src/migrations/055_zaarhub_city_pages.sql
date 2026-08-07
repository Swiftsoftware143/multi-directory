-- 027_zaarhub_city_pages: City Pages & Business Listings for ZaarHub Hub
-- Phase 4 — Dynamic city directory pages served by FunnelSwift

CREATE TABLE IF NOT EXISTS city_pages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    city_slug       VARCHAR(255) NOT NULL,
    city_name       VARCHAR(255) NOT NULL,
    state           VARCHAR(50),
    description     TEXT,
    hero_image_url  TEXT,
    meta_title      VARCHAR(255),
    meta_description TEXT,
    is_active       BOOLEAN NOT NULL DEFAULT true,
    display_order   INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_city_pages_tenant ON city_pages(tenant_id);
CREATE INDEX idx_city_pages_active ON city_pages(is_active) WHERE is_active = true;
CREATE UNIQUE INDEX idx_city_pages_slug ON city_pages(tenant_id, city_slug);

CREATE TABLE IF NOT EXISTS business_listings (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    city_page_id    UUID NOT NULL REFERENCES city_pages(id) ON DELETE CASCADE,
    business_name   VARCHAR(255) NOT NULL,
    category        VARCHAR(255),
    subcategory     VARCHAR(255),
    description     TEXT,
    address         VARCHAR(255),
    phone           VARCHAR(50),
    website         VARCHAR(500),
    logo_url        TEXT,
    cover_image_url TEXT,
    rating          DOUBLE PRECISION DEFAULT 0,
    review_count    INTEGER NOT NULL DEFAULT 0,
    is_featured     BOOLEAN NOT NULL DEFAULT false,
    is_claimed      BOOLEAN NOT NULL DEFAULT false,
    deal_text       TEXT,
    deal_url        TEXT,
    coordinates_lat DOUBLE PRECISION,
    coordinates_lng DOUBLE PRECISION,
    display_order   INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_business_listings_city ON business_listings(city_page_id);
CREATE INDEX idx_business_listings_category ON business_listings(category);
CREATE INDEX idx_business_listings_featured ON business_listings(is_featured) WHERE is_featured = true;
CREATE INDEX idx_business_listings_rating ON business_listings(rating DESC);
CREATE INDEX idx_business_listings_name ON business_listings(business_name);
CREATE INDEX idx_business_listings_search ON business_listings
    USING gin (to_tsvector('english', coalesce(business_name,'') || ' ' || coalesce(description,'') || ' ' || coalesce(category,'')));
