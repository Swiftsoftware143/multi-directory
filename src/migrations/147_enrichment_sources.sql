-- B83 part 1 — the Sources panel: a per-source on/off set for the enrichment cycle.
--
-- Before this column the cycle took a single `provider` pin (or auto-selected one). David's
-- ask (2026-09-30) is a simple CHECKBOX PER SOURCE, with sensible defaults: a keyless source
-- (OpenStreetMap) is on out of the box, and a keyed source switches itself on automatically
-- once a valid key is saved. `enabled_sources` records the admin's explicit choice; NULL means
-- "use the defaults" (keyless sources on, keyed sources on when a key is configured), so an
-- untouched directory behaves exactly as before.
--
-- Idempotent: safe to re-run.
ALTER TABLE enrichment_settings
    ADD COLUMN IF NOT EXISTS enabled_sources text[];
