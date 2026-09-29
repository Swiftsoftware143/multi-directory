-- 104: make "one default key per provider" scope-aware.
--
-- The pre-scope rule was one default per provider, period. Now that a key can be scoped to a
-- network or a directory (migration 103), that rule is wrong: a global default and a network-level
-- key for the same provider are BOTH legitimate, and the old index rejected the second one with
-- 'duplicate key value violates unique constraint "provider_keys_one_default_per_provider"' — which
-- is what blocked saving David's Google key (provider_keys_handler returns 500 on that).
--
-- New rule: one default per (provider, scope, target) — where the target is the network or the
-- directory the key is scoped to, and empty for a global/tenant key.

DROP INDEX IF EXISTS provider_keys_one_default_per_provider;

CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_provider_scope
    ON provider_keys (
        provider,
        COALESCE(scope, 'global'),
        COALESCE(network_id::text, ''),
        COALESCE(directory_id::text, '')
    )
    WHERE is_default;

-- Older rows written before the scope columns existed may carry NULL scope; normalise so the
-- resolution order (directory -> network -> global) sees a real value instead of a NULL mismatch.
UPDATE provider_keys SET scope = 'global' WHERE scope IS NULL;
