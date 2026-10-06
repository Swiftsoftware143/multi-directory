-- 161: Stripe Connect onboarding for business payouts (card B143).
--
-- The clearinghouse (settlement.rs) reimburses a redeeming business with a Stripe transfer,
-- but until now the money could only go to ONE network-level destination
-- (point_treasury.payout_destination, migration 160) — i.e. the operator's own account. To pay
-- each business on its own Stripe account the platform needs Stripe Connect: the owner onboards
-- through Stripe and the resulting connected account id is stored per business so settlement can
-- route the transfer to it.
--
-- These columns hold that per-business Connect state. They are written ONLY from the business
-- portal (/portal/business/payouts/*), never by an operator in SQL. The defaults read as
-- "not connected", so every existing row is safe and the feature degrades honestly when Stripe
-- is not configured.

ALTER TABLE businesses ADD COLUMN IF NOT EXISTS stripe_connect_account_id TEXT;
ALTER TABLE businesses
    ADD COLUMN IF NOT EXISTS stripe_connect_status TEXT NOT NULL DEFAULT 'not_connected';
ALTER TABLE businesses
    ADD COLUMN IF NOT EXISTS stripe_payouts_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS stripe_connect_updated_at TIMESTAMPTZ;

-- status is a closed set: a typo must fail the write rather than silently invent a new state.
ALTER TABLE businesses DROP CONSTRAINT IF EXISTS businesses_stripe_connect_status_check;
ALTER TABLE businesses ADD CONSTRAINT businesses_stripe_connect_status_check
    CHECK (stripe_connect_status IN ('not_connected', 'pending', 'restricted', 'connected'));
