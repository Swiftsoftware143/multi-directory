-- 213_service_prices_checkout_embeds.sql
--
-- Add-ons (service_prices) are sold standalone, monthly and yearly, exactly like plan tiers, so they
-- need the same two checkout embeds (ScalePay's priceConfig holds one price object → one embed per
-- billing cycle). Mirrors plan_tiers.checkout_embed_monthly/_yearly (migration 212).
--
-- Idempotent: guarded by IF NOT EXISTS so a re-run / re-boot applies nothing twice.

ALTER TABLE service_prices ADD COLUMN IF NOT EXISTS checkout_embed_monthly text;
ALTER TABLE service_prices ADD COLUMN IF NOT EXISTS checkout_embed_yearly text;

COMMENT ON COLUMN service_prices.checkout_embed_monthly IS
    'Embeddable checkout for buying this add-on MONTHLY: iframe URL or embed snippet rendered inline on the purchase screen. NULL = none.';
COMMENT ON COLUMN service_prices.checkout_embed_yearly IS
    'Embeddable checkout for buying this add-on YEARLY: iframe URL or embed snippet rendered inline on the purchase screen. NULL = none.';
