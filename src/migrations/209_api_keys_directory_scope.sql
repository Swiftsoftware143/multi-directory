-- 209 — per-directory API credentials (kanban t_6a165e41).
--
-- A directory-scoped key records WHICH directory it belongs to in `directory_id`. Until now
-- `api_keys.tenant_id` named a CUSTOMER tenant (`tenants.id`) and could never answer "which
-- directory does this key see?" — the resolver had no directory to filter on. That is the
-- tenant->directory mapping this card fixes.
--
--   * operator key  — `directory_id IS NULL`, owner `super_admin` — sees every directory;
--   * directory key — `directory_id` set, minted automatically when its directory is created —
--     sees only that directory's listings/sponsors/ads.
--
-- The FK is ON DELETE CASCADE so deleting a directory removes its key in the same transaction:
-- a deleted directory can never leave a working credential behind.

ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS directory_id uuid REFERENCES directories(id) ON DELETE CASCADE;

CREATE INDEX IF NOT EXISTS idx_api_keys_directory_id
    ON api_keys (directory_id) WHERE directory_id IS NOT NULL;
