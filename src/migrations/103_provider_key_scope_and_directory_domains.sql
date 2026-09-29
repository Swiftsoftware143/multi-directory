-- 103: provider-key scope (network / directory inheritance) + a domain per directory.
--
-- FEATURE 1 (David, 2026-09-29): an API key belongs to the PLATFORM, to one NETWORK, or to one
-- DIRECTORY. A directory with no key of its own INHERITS its network's key, so a Google Places
-- key is set ONCE for the ZaarHub network and all ten cities use it. A standalone directory
-- (network_id IS NULL) keeps its own key. Resolution order is directory -> network -> platform,
-- implemented once in provider_keys_handler::resolve_provider_key_scoped (no ad-hoc SQL anywhere).
--
-- FEATURE 2: one directory may be served on its own custom host, on a subdomain of its network's
-- root domain, or in a subfolder of a host (zaarhub.com/palm-bay). domain_mappings.domain's
-- UNIQUE(domain) made two directories sharing one host impossible, so it is replaced by a
-- (host, path) uniqueness.

-- ── Feature 1 ────────────────────────────────────────────────────────────────────────────────
ALTER TABLE provider_keys ADD COLUMN IF NOT EXISTS network_id uuid;
ALTER TABLE provider_keys ADD COLUMN IF NOT EXISTS directory_id uuid;
ALTER TABLE provider_keys DROP CONSTRAINT IF EXISTS provider_keys_network_id_fkey;
ALTER TABLE provider_keys ADD CONSTRAINT provider_keys_network_id_fkey FOREIGN KEY (network_id) REFERENCES networks (id) ON DELETE CASCADE;
ALTER TABLE provider_keys DROP CONSTRAINT IF EXISTS provider_keys_directory_id_fkey;
ALTER TABLE provider_keys ADD CONSTRAINT provider_keys_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES directories (id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_provider_keys_network ON provider_keys (network_id, provider);
CREATE INDEX IF NOT EXISTS idx_provider_keys_directory ON provider_keys (directory_id, provider);
CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_network ON provider_keys (network_id, provider) WHERE is_default = true AND network_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_directory ON provider_keys (directory_id, provider) WHERE is_default = true AND directory_id IS NOT NULL;
-- A key is scoped to at most ONE place: the platform, one network, or one directory. A row with
-- both columns set would be ambiguous for the resolver, so the database refuses it.
ALTER TABLE provider_keys DROP CONSTRAINT IF EXISTS provider_keys_single_scope;
ALTER TABLE provider_keys ADD CONSTRAINT provider_keys_single_scope CHECK (NOT (network_id IS NOT NULL AND directory_id IS NOT NULL));

-- ── Feature 2 ────────────────────────────────────────────────────────────────────────────────
ALTER TABLE domain_mappings DROP CONSTRAINT IF EXISTS domain_mappings_domain_key;
ALTER TABLE domain_mappings ADD COLUMN IF NOT EXISTS url_path text NOT NULL DEFAULT '';
ALTER TABLE domain_mappings ADD COLUMN IF NOT EXISTS live_status text;
ALTER TABLE domain_mappings ADD COLUMN IF NOT EXISTS last_checked_at timestamp with time zone;
ALTER TABLE domain_mappings ADD COLUMN IF NOT EXISTS last_check_detail text;
CREATE UNIQUE INDEX IF NOT EXISTS domain_mappings_host_path ON domain_mappings (domain, url_path);
CREATE INDEX IF NOT EXISTS idx_domain_mappings_live ON domain_mappings (status, live_status);
