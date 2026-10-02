-- 072_provider_keys.sql: create provider_keys (referenced but never defined
-- in any earlier migration; 038 assumed it existed, so it was silently missing).

CREATE TABLE IF NOT EXISTS provider_keys (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID NOT NULL,
    provider         TEXT NOT NULL,
    api_key          TEXT,
    base_url         TEXT,
    metadata         JSONB NOT NULL DEFAULT '{}',
    is_active        BOOLEAN NOT NULL DEFAULT true,
    scope            TEXT NOT NULL DEFAULT 'tenant',
    api_key_encrypted BYTEA,
    base_url_encrypted BYTEA,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, provider)
);

CREATE INDEX IF NOT EXISTS idx_provider_keys_tenant ON provider_keys(tenant_id);
