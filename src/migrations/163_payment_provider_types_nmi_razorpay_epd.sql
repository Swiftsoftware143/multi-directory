-- ============================================================
-- MultiDirectory — 163: Integration Center payment gateways — NMI / Razorpay / EPD
-- (2026-10-08)
--
-- David's Integration Center card (B130): the payment providers he names — Stripe, NMI, EPD,
-- Razorpay — must be selectable in ONE admin screen alongside the provider API keys.
-- Before this, `payment_providers.provider_type` was CHECK-limited to
-- ('stripe','paypal','square','paddle'), so a request for any of the three named gateways was
-- rejected by the database as well as by the handler.
--
-- HONESTY NOTE: only `stripe` and `paypal` have a live webhook receiver today. `square`,
-- `paddle`, `nmi`, `razorpay` and `epd` are selectable so the operator can RECORD the gateway
-- they intend to use and see it in the Integration Center; the admin panel labels them
-- "(no receiver yet)" and will not let one be armed. Nothing here pretends a receiver exists.
--
-- Idempotent: DROP ... IF EXISTS then ADD — src/db.rs::run_migrations re-runs every file in
-- src/migrations on each boot, so the second boot drops and re-adds the same constraint.
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
        'razorpay'::text,
        'epd'::text
    ]));
