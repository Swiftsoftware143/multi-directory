-- Stage 5: Business Service Catalog
-- Per-business services/products that visitors can browse and book

CREATE TABLE IF NOT EXISTS business_services (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    price NUMERIC(10,2),
    currency TEXT NOT NULL DEFAULT 'USD',
    duration_minutes INTEGER,
    category TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_business_services_business ON business_services(business_id);
CREATE INDEX IF NOT EXISTS idx_business_services_directory ON business_services(directory_id);
CREATE INDEX IF NOT EXISTS idx_business_services_active ON business_services(business_id) WHERE is_active = true;

-- Trigger to auto-update updated_at
CREATE OR REPLACE FUNCTION update_business_services_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_business_services_updated_at ON business_services;
CREATE TRIGGER trg_business_services_updated_at
    BEFORE UPDATE ON business_services
    FOR EACH ROW
    EXECUTE FUNCTION update_business_services_updated_at();

INSERT INTO _migrations (filename) VALUES ('036_service_catalog.sql');
