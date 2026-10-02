-- ============================================================
-- MultiDirectory — 091: Integration Center (catalogue + CoreSwift lead push)
-- (Sept 20 2026)
--
-- Fleet standard: /opt/swift/docs/integration-center-standard-2026-09-20.md
--   R1 every app has an Integration Center
--   R2 every app has a NATIVE CoreSwift integration whose data flows DOWNWARD
--      into CoreSwift (this app CAPTURES leads; CoreSwift is the hub / single home).
--
--   1. Assert the canonical `coreswift` catalogue row. Migration 077 seeded it with
--      requires_base_url = TRUE, which contradicts the fleet standard
--      (requires_base_url = FALSE, description = 'Push leads into CoreSwift CRM').
--      The base URL is resolved in code
--      (provider_keys.base_url -> integration_provider_presets.base_url -> default),
--      so the tenant is never asked for it in the Integration Center.
--   2. `integration_provider_presets` — step 2 of the documented base-URL resolution
--      order, with the coreswift preset row (http://localhost:8084 = the on-box
--      crm-swift hub) so no spoke hardcodes the hub URL.
--
-- Idempotent (IF NOT EXISTS / ON CONFLICT / plain UPDATE) — src/db.rs::run_migrations
-- re-runs every file in src/migrations on each boot.
-- ============================================================

-- 1) Canonical CoreSwift catalogue row (identical in every spoke) -------------
UPDATE available_providers
SET name = 'CoreSwift CRM',
    description = 'Push leads into CoreSwift CRM',
    requires_base_url = false,
    requires_metadata = '[]'::jsonb
WHERE key = 'coreswift';

-- Defensive: if the row is missing entirely (fresh DB), seed it canonically.
INSERT INTO available_providers (key, name, description, requires_base_url, requires_metadata, icon)
SELECT 'coreswift', 'CoreSwift CRM', 'Push leads into CoreSwift CRM', false, '[]'::jsonb, 'crm'
WHERE NOT EXISTS (SELECT 1 FROM available_providers WHERE key = 'coreswift');

-- 2) Provider presets (base-URL resolution step 2) ---------------------------
CREATE TABLE IF NOT EXISTS integration_provider_presets (
    key        TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    base_url   TEXT NOT NULL DEFAULT '',
    docs_url   TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active  BOOLEAN NOT NULL DEFAULT true
);

INSERT INTO integration_provider_presets (key, name, base_url, docs_url, sort_order, is_active)
VALUES
    ('coreswift', 'CoreSwift CRM', 'http://localhost:8084', 'https://coreswiftcrm.com/docs', -10, true)
ON CONFLICT (key) DO UPDATE
SET name = EXCLUDED.name,
    base_url = EXCLUDED.base_url,
    docs_url = EXCLUDED.docs_url,
    is_active = true;
