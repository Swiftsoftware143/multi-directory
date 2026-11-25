-- 215_business_placement_type.sql
--
-- Distinguish two kinds of non-home placement (work-queue t_5306e167 follow-up):
--
--   satellite        auto-computed MARKETING reach (Featured: same-region cities; Premium: every
--                    city), capped by plan_tiers.max_listings. Re-computed (deleted + re-inserted)
--                    by entitlements::sync_placements whenever a subscription changes.
--
--   secondary_office a REAL physical location in another city, added MANUALLY by a verified
--                    employee. NOT limited by region and NOT capped by max_listings — a business
--                    that genuinely operates in two cities must appear in both. sync_placements
--                    PRESERVES these rows (it never deletes placement_type = 'secondary_office'),
--                    so a real branch is never cheated out of its listing.
--
-- The public listing query (entitlements::placement_visible) matches on ANY row regardless of
-- type, so both marketing satellites and real secondary offices surface the business in that city.

ALTER TABLE business_placements
    ADD COLUMN IF NOT EXISTS placement_type text NOT NULL DEFAULT 'satellite';

-- Allowed values, idempotent (drop then add so a re-run / partial state can't leave a stale pair).
DO $$
BEGIN
    ALTER TABLE business_placements
        DROP CONSTRAINT IF EXISTS business_placements_type_check;
    ALTER TABLE business_placements
        ADD CONSTRAINT business_placements_type_check
        CHECK (placement_type IN ('satellite', 'secondary_office'));
END $$;

COMMENT ON COLUMN business_placements.placement_type IS
    'satellite = auto marketing placement (capped by max_listings); secondary_office = real branch, manual, preserved.';
