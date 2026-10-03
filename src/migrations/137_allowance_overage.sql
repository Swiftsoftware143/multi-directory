-- B114: allowance + metered overage funding model (merged — David + engineer 2026-10-02).
--
-- Each plan tier's `monthly_point_allowance` is prepaid by the subscription, so points
-- issued WITHIN the allowance are already funded and must not be billed again. Only the
-- points issued BEYOND the allowance (the overage) are invoiced, at overage_rate_per_point
-- and capped by overage_cap_points.
--
-- overage_mode:
--   'bill'  — consented overage billing. DEFAULT, and because the default rate equals the
--             issuance rate (0.01) behaviour is unchanged for a business with no plan.
--   'pause' — earning stops at the allowance; nothing beyond it is ever issued or billed.
--
-- Every value is admin-configurable per network (Clearinghouse settings), never a code
-- constant, so a buyer can size their own economics without a deploy.
ALTER TABLE point_treasury
    ADD COLUMN IF NOT EXISTS overage_rate_per_point numeric(12,6) NOT NULL DEFAULT 0.010000;
ALTER TABLE point_treasury
    ADD COLUMN IF NOT EXISTS overage_mode varchar(16) NOT NULL DEFAULT 'bill';
ALTER TABLE point_treasury
    ADD COLUMN IF NOT EXISTS overage_cap_points integer;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'point_treasury_overage_mode_check'
    ) THEN
        ALTER TABLE point_treasury
            ADD CONSTRAINT point_treasury_overage_mode_check
            CHECK (overage_mode IN ('bill', 'pause'));
    END IF;
END $$;

-- Per-invoice breakdown so a settlement statement can show what the allowance covered.
ALTER TABLE settlement_invoices
    ADD COLUMN IF NOT EXISTS allowance_points bigint NOT NULL DEFAULT 0;
ALTER TABLE settlement_invoices
    ADD COLUMN IF NOT EXISTS overage_points bigint NOT NULL DEFAULT 0;
