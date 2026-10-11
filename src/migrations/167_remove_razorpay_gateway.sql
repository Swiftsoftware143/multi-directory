-- ============================================================
-- MultiDirectory — 167: payment gateways — drop Razorpay (India-only)
-- (2026-10-10)
--
-- David's MD STRUCTURE rule (2026-10-10): "Payments: keep Stripe, add NMI/EPD as panel
-- options, SKIP Razorpay (India-only)". Migration 163 (2026-10-08) had added `razorpay`
-- to the selectable set; this file supersedes that and removes it, so the Integration
-- Center no longer offers a gateway this US directory can never use.
--
-- Idempotent: DROP ... IF EXISTS then ADD — src/db.rs::run_migrations re-runs every file
-- in src/migrations on each boot. No payment_providers row used `razorpay` (verified 0
-- rows on live), so the tightened constraint validates cleanly.
-- ============================================================

ALTER TABLE payment_providers
    DROP CONSTRAINT IF EXISTS payment_providers_provider_type_check;

ALTER TABLE payment_providers
    ADD CONSTRAINT payment_providers_provider_type_check
    CHECK (provider_type = ANY (ARRAY[
        'stripe'::text,
        'paypal'::text,
        'square'::text,
        'paddle'::text,
        'nmi'::text,
        'epd'::text
    ]));
