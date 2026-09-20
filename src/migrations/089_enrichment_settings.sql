-- Migration 089: rotating enrichment-cycle settings (T4).
--
-- data_enrichment_logs exists but holds 0 rows: nothing ever runs on a schedule.
-- This holds the cadence/batch/enable knobs the admin Data Enrichment card edits.
-- directory_id IS NULL is the global default; a directory may override it.
--
-- The provider is deliberately NULL by default: NULL means "use the first
-- configured provider from provider_keys" so no vendor is baked in. When nothing
-- is configured the run logs a skip and writes nothing — it never panics and never
-- pretends work happened.

CREATE TABLE IF NOT EXISTS enrichment_settings (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id  uuid,
    is_enabled    boolean NOT NULL DEFAULT false,
    cadence_hours integer NOT NULL DEFAULT 24,
    batch_size    integer NOT NULL DEFAULT 25,
    provider      varchar(64),
    last_run_at   timestamptz,
    last_status   varchar(32),
    next_run_at   timestamptz,
    updated_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT enrichment_settings_cadence_check CHECK (cadence_hours >= 1),
    CONSTRAINT enrichment_settings_batch_check CHECK (batch_size >= 1 AND batch_size <= 500)
);

CREATE INDEX IF NOT EXISTS idx_enrichment_settings_directory_id ON enrichment_settings(directory_id);
CREATE INDEX IF NOT EXISTS idx_enrichment_settings_next_run_at ON enrichment_settings(next_run_at);

-- Seed the global default row exactly once. Disabled by default: enabling it is an
-- explicit admin decision in the panel.
INSERT INTO enrichment_settings (directory_id, is_enabled, cadence_hours, batch_size)
SELECT NULL, false, 24, 25
WHERE NOT EXISTS (SELECT 1 FROM enrichment_settings WHERE directory_id IS NULL);
