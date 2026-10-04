-- 154_business_trust_badges.sql — "Licensed & insured" trust badges on a listing (card B90,
-- Angie's List-style competitive parity).
--
-- David's competitive-parity mandate (2026-10-01) approves verified/licensed/insured badges per
-- business. `businesses.verified` already exists and drives the public "Verified" badge, but there
-- was no way to record that the operator has checked a business's trade licence or its insurance
-- cover — Angie's List and Thumbtack both surface that as the primary trust signal on a listing.
--
-- Three columns, all admin-settable from the Businesses tab of the admin panel (card B84: the
-- buyer runs this without SQL):
--   licensed        — the operator has seen a trade licence (shown as a public badge).
--   insured         — the operator has seen proof of insurance (shown as a public badge).
--   license_number  — the licence reference, optional, operator-only (never rendered publicly).
--
-- Booleans default FALSE and are NOT NULL so every existing row is unambiguously "not yet
-- verified" rather than NULL-of-unknown-meaning. Idempotent: safe on a fresh install and safe to
-- re-run on a live DB.

ALTER TABLE public.businesses ADD COLUMN IF NOT EXISTS licensed       boolean NOT NULL DEFAULT false;
ALTER TABLE public.businesses ADD COLUMN IF NOT EXISTS insured        boolean NOT NULL DEFAULT false;
ALTER TABLE public.businesses ADD COLUMN IF NOT EXISTS license_number text;
