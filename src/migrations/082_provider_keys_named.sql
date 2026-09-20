-- Migration 082: named provider keys (T0).
-- Today provider_keys is UNIQUE (tenant_id, provider) => exactly one key per service.
-- David needs several keys of the same service, each with its own human name
-- ("Palm Bay project", "Test account") and a default that resolution picks first.

ALTER TABLE provider_keys ADD COLUMN IF NOT EXISTS label TEXT;
UPDATE provider_keys SET label = 'default' WHERE label IS NULL;
ALTER TABLE provider_keys ALTER COLUMN label SET DEFAULT 'default';
ALTER TABLE provider_keys ALTER COLUMN label SET NOT NULL;

-- Existing rows were the only key for their provider => they are the default.
ALTER TABLE provider_keys ADD COLUMN IF NOT EXISTS is_default BOOLEAN NOT NULL DEFAULT false;
UPDATE provider_keys SET is_default = true WHERE NOT EXISTS (
    SELECT 1 FROM provider_keys o
    WHERE o.tenant_id = provider_keys.tenant_id
      AND o.provider = provider_keys.provider
      AND o.is_default = true
);

-- One key per (tenant, provider, label); many labels per provider now.
ALTER TABLE provider_keys DROP CONSTRAINT IF EXISTS provider_keys_tenant_id_provider_key;
ALTER TABLE provider_keys DROP CONSTRAINT IF EXISTS provider_keys_tenant_id_provider_label_key;
ALTER TABLE provider_keys ADD CONSTRAINT provider_keys_tenant_id_provider_label_key
    UNIQUE (tenant_id, provider, label);

-- Resolution helper index: default first, then most recently updated.
CREATE INDEX IF NOT EXISTS idx_provider_keys_resolve
    ON provider_keys (tenant_id, provider, is_default DESC, updated_at DESC);

-- Only one default per (tenant, provider) — enforced so "the key" is deterministic.
CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_provider
    ON provider_keys (tenant_id, provider) WHERE is_default = true;
