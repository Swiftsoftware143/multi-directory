-- 212_plan_checkout_embed_monthly_yearly.sql
--
-- Split the single plan checkout embed (211) into TWO — one per billing cycle — because ScalePay's
-- priceConfig holds ONE price object: a monthly and a yearly price cannot share one checkout, so each
-- tier is two campaigns. A buyer choosing monthly gets checkout_embed_monthly, yearly gets
-- checkout_embed_yearly.
--
-- Idempotent: ADD COLUMN IF NOT EXISTS + DROP COLUMN IF EXISTS; the backfill UPDATE is guarded so a
-- re-run / re-boot applies nothing twice. (checkout_embed was brand-new in 211 and unset in practice.)

ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS checkout_embed_monthly text;
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS checkout_embed_yearly text;

-- Carry any value that was already pasted into the single field across to the monthly slot.
UPDATE plan_tiers
   SET checkout_embed_monthly = checkout_embed
 WHERE checkout_embed_monthly IS NULL AND checkout_embed IS NOT NULL;

ALTER TABLE plan_tiers DROP COLUMN IF EXISTS checkout_embed;

COMMENT ON COLUMN plan_tiers.checkout_embed_monthly IS
    'Embeddable checkout for buying this plan MONTHLY: iframe URL or embed snippet rendered inline on the purchase screen. NULL = none.';
COMMENT ON COLUMN plan_tiers.checkout_embed_yearly IS
    'Embeddable checkout for buying this plan YEARLY: iframe URL or embed snippet rendered inline on the purchase screen. NULL = none.';
