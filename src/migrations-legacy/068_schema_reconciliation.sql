-- 068_schema_reconciliation: restore missing tables/columns the codebase expects.
-- The canonical model uses `directories` (not `tenants`), plus `crm_contacts`,
-- and `claimed_businesses` needs the business_id/owner_* columns.

-- ── 1. claimed_businesses: add the columns code reads/writes ──────────────────
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS business_id UUID REFERENCES businesses(id) ON DELETE CASCADE;
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS owner_email VARCHAR(255);
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS owner_name VARCHAR(255);
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS owner_phone VARCHAR(50);
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS verified_at TIMESTAMPTZ;
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS last_dashboard_login TIMESTAMPTZ;
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE UNIQUE INDEX IF NOT EXISTS idx_claimed_businesses_business_id
    ON claimed_businesses (business_id) WHERE business_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_claimed_businesses_owner_email
    ON claimed_businesses (owner_email);

-- ── 2. crm_contacts table ────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS crm_contacts (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    first_name        VARCHAR(100),
    last_name         VARCHAR(100),
    email             VARCHAR(255),
    phone             VARCHAR(50),
    company           VARCHAR(255),
    position          VARCHAR(100),
    directory_id      UUID,
    status            VARCHAR(50) DEFAULT 'new',
    tags              TEXT[],
    notes             TEXT,
    source            VARCHAR(100),
    assigned_to       UUID,
    last_contacted_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_crm_contacts_directory ON crm_contacts(directory_id);
CREATE INDEX IF NOT EXISTS idx_crm_contacts_email ON crm_contacts(email);

-- ── 3. directories table (canonical model — see models/directory.rs) ─────────
CREATE TABLE IF NOT EXISTS directories (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name                 VARCHAR(255) NOT NULL,
    slug                 VARCHAR(100) NOT NULL,
    description          TEXT,
    status               VARCHAR(50) DEFAULT 'draft',
    owner_id             UUID,
    template             VARCHAR(64) DEFAULT 'local-business',
    color_scheme         JSONB,
    network_id           UUID,
    url_type             VARCHAR(20) DEFAULT 'standalone',
    url_value            VARCHAR(255),
    custom_domain        VARCHAR(255),
    city                 VARCHAR(100),
    template_config      JSONB,
    tracking_enabled     BOOLEAN DEFAULT true,
    feature_config       JSONB,
    head_injection       TEXT,
    body_injection       TEXT,
    footer_injection     TEXT,
    email_signature_html TEXT,
    email_signature_text TEXT,
    zaarhub_config       JSONB,
    api_config           JSONB DEFAULT '{}'::jsonb,
    coreswift_tenant_id  UUID,
    coreswift_key_prefix TEXT,
    coreswift_list_id_sponsors   UUID,
    coreswift_list_id_claimed    UUID,
    coreswift_list_id_newsletter UUID,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_directories_slug ON directories(slug);
CREATE INDEX IF NOT EXISTS idx_directories_network ON directories(network_id) WHERE network_id IS NOT NULL;

-- ── 4. businesses: add columns the code reads/writes ─────────────────────────
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS directory_id UUID;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS category_id UUID;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS business_type VARCHAR(100);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS city VARCHAR(100);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS state VARCHAR(100);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS zip VARCHAR(20);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS latitude DOUBLE PRECISION;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS longitude DOUBLE PRECISION;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS images JSONB;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS search_vector TSVECTOR;
CREATE INDEX IF NOT EXISTS idx_businesses_directory ON businesses(directory_id) WHERE directory_id IS NOT NULL;

-- Seed a default directory from the existing tenant if present, to keep the
-- host-routing and reminders paths from being empty.
INSERT INTO directories (name, slug, description, status, network_id, created_at, updated_at)
SELECT t.name, t.slug, t.name, 'active', n.id, t.created_at, t.updated_at
FROM tenants t
LEFT JOIN networks n ON n.slug = t.slug
ON CONFLICT DO NOTHING;
