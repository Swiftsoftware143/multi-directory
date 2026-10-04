-- 155_float_rule_settings.sql — the ZaarCash float rule: three conditions, HOLD if any fails.
--
-- David, 2026-10-04: "You come up with best industry practice float rule. But again that should be
-- able to be configured also by the Admin in the admin panel."
--
-- Until now the float was ONE flat comparison — `position >= minimum_float` (and only in the
-- console's own arithmetic). One number cannot adapt: a programme with a hundred ZaarCash
-- outstanding and one with a million need the same RELATIONSHIP held, not the same dollar figure.
-- The rule is now three independent conditions and the float is held when any of them fails:
--
--   1. COVERAGE  available >= float_coverage_pct% x outstanding_liability
--                (accounting view — never redeem points nobody funded)
--   2. BURN      available >= float_burn_months x trailing-30-day redemption volume
--                (operational view — a programme can be fully covered on paper and still fail
--                 when several members redeem at once)
--   3. FLOOR     available >= minimum_float
--                (risk view — a brand-new programme has near-zero liability AND near-zero burn,
--                 so it would pass the other two while holding nothing)
--
-- Two new columns; `minimum_float` (already numeric(14,2) NOT NULL DEFAULT 100.00) IS the floor,
-- so no data migration and no rename — existing rows keep working untouched.
--
-- Defaults are best-practice values, not constants baked into code: 100% coverage, ONE month of
-- burn, $100 floor. All three are editable in the admin panel's Clearinghouse card, and a value
-- that is missing, unparseable or out of range falls back to the conservative default at
-- evaluation time (see src/handlers/float_rule.rs) — a bad setting fails safe, never open.
--
-- The CHECK constraints mirror the read-side clamp so a hand-written SQL value cannot store a
-- policy the engine refuses to honour. 0 is a legal value for all three: it means "this condition
-- is off", which is how an operator deliberately runs an unfunded pilot. Idempotent: safe on a
-- fresh install and safe to re-run on a live DB.

ALTER TABLE public.point_treasury
    ADD COLUMN IF NOT EXISTS float_coverage_pct numeric(6,2) NOT NULL DEFAULT 100.00;

ALTER TABLE public.point_treasury
    ADD COLUMN IF NOT EXISTS float_burn_months  numeric(6,2) NOT NULL DEFAULT 1.00;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'point_treasury_float_coverage_pct_check'
    ) THEN
        ALTER TABLE public.point_treasury
            ADD CONSTRAINT point_treasury_float_coverage_pct_check
            CHECK (float_coverage_pct >= 0 AND float_coverage_pct <= 100);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'point_treasury_float_burn_months_check'
    ) THEN
        ALTER TABLE public.point_treasury
            ADD CONSTRAINT point_treasury_float_burn_months_check
            CHECK (float_burn_months >= 0 AND float_burn_months <= 12);
    END IF;
END $$;
