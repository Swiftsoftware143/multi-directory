-- 204_incentive_widget.sql
--
-- Ecosystem widgets, increment 2 of 2 (work-queue "ECOSYSTEM WIDGETS", David's priority
-- "before loyalty, after market-ready"). Per business, an IncentiveSwift campaign widget so a
-- visitor captures an offer INSIDE the directory instead of being hyperlinked off-site.
-- Increment 1 shipped the CoreSwift booking calendar (`booking_widget_enabled`, migration 203);
-- this is the IncentiveSwift campaign half. `incentive_widget_enabled` is the per-business switch
-- the operator flips in the admin panel; `incentive_campaign_slug` is the business's external id
-- in IncentiveSwift (the campaign slug). A business with the flag off (or a blank slug) renders
-- NO widget and no empty box.
--
-- Idempotent: guarded by IF NOT EXISTS so a re-run / re-boot applies nothing twice.

ALTER TABLE businesses ADD COLUMN IF NOT EXISTS incentive_widget_enabled boolean NOT NULL DEFAULT false;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS incentive_campaign_slug text;
