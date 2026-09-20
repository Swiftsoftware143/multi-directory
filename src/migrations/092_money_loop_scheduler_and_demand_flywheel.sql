-- Migration 092: the money loop closes by itself, and unmet demand becomes supply.
--
-- Round 14, two halves.
--
-- T1 (scheduler):  `settlement_runs` gains the bookkeeping the panel needs to show
--   what the scheduler did (how the run fired, and whether statements went out).
--   `point_issuance_log` gains `expired_at` so point expiry is IDEMPOTENT: the
--   existing expiry path summed every issuance older than the cutoff and subtracted
--   it from the member balance on every call, so a scheduler waking hourly would
--   have drained balances. Rows are now marked expired as they are consumed, and a
--   repeat run finds nothing left to consume.
--   `point_treasury.expiry_last_run_at` records the last expiry pass per network.
--
-- T2 (flywheel):  `discovery_queue` gains a `source` marker ('admin_search' for the
--   existing populate flow, 'unmet_demand' for rows derived from zero-result
--   searches) plus the observed search volume and area, so the review queue shows
--   where each row came from. The flywheel's threshold and cap are DATA on
--   `demand_analytics_settings` — never constants in code.

-- ── T1: idempotent point expiry ────────────────────────────────────────────────
-- NULL = these issued points have not been consumed by an expiry pass yet.
ALTER TABLE point_issuance_log ADD COLUMN IF NOT EXISTS expired_at timestamptz;

-- The expiry pass scans unexpired rows older than the cutoff; this partial index
-- keeps that scan off the full log as it grows.
CREATE INDEX IF NOT EXISTS idx_pil_unexpired_issuance
    ON point_issuance_log (network_id, created_at) WHERE expired_at IS NULL;

-- ── T1: settlement run bookkeeping ─────────────────────────────────────────────
-- 'manual' | 'scheduler' — the panel shows how the run actually fired.
ALTER TABLE settlement_runs ADD COLUMN IF NOT EXISTS triggered_by varchar(16) NOT NULL DEFAULT 'manual';

-- Statement delivery is recorded per run: a run that emailed nobody because the
-- directory has no provider configured says so, instead of pretending it sent.
ALTER TABLE settlement_runs ADD COLUMN IF NOT EXISTS statements_sent_at timestamptz;
ALTER TABLE settlement_runs ADD COLUMN IF NOT EXISTS statements_sent integer NOT NULL DEFAULT 0;
ALTER TABLE settlement_runs ADD COLUMN IF NOT EXISTS statements_skipped integer NOT NULL DEFAULT 0;
ALTER TABLE settlement_runs ADD COLUMN IF NOT EXISTS statements_message text;

-- Last point-expiry pass per network (policy-driven, driven by the scheduler).
ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS expiry_last_run_at timestamptz;

-- ── T2: discovery queue provenance ─────────────────────────────────────────────
-- Existing rows are admin-driven searches; that stays the default so the current
-- populate flow is unchanged.
ALTER TABLE discovery_queue ADD COLUMN IF NOT EXISTS source text NOT NULL DEFAULT 'admin_search';
-- Observed zero-result search volume behind an 'unmet_demand' row (NULL for admin rows).
ALTER TABLE discovery_queue ADD COLUMN IF NOT EXISTS demand_volume integer;
-- The area (zip or city) the unmet searches came from — the "where" of the demand.
ALTER TABLE discovery_queue ADD COLUMN IF NOT EXISTS demand_area text;

CREATE INDEX IF NOT EXISTS idx_discovery_queue_source
    ON discovery_queue (directory_id, source);

-- ── T2: flywheel thresholds are settings, not constants ────────────────────────
-- A zero-result term is only worth chasing once it has been searched at least this
-- many times; the cap bounds one click's worth of queue rows.
ALTER TABLE demand_analytics_settings ADD COLUMN IF NOT EXISTS unmet_min_searches integer NOT NULL DEFAULT 2;
ALTER TABLE demand_analytics_settings ADD COLUMN IF NOT EXISTS unmet_queue_limit integer NOT NULL DEFAULT 25;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'demand_analytics_settings_unmet_check'
    ) THEN
        ALTER TABLE demand_analytics_settings
            ADD CONSTRAINT demand_analytics_settings_unmet_check
            CHECK (unmet_min_searches > 0 AND unmet_queue_limit > 0);
    END IF;
END $$;
