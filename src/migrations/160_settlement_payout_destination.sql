-- 160: settlement payout destination on the network treasury row.
--
-- The clearinghouse payout path (settlement.rs) sends Stripe transfers to a
-- "payout destination" (a Stripe connected account id). Until now that value could
-- only live in provider_keys.metadata->>'payout_destination', which has no admin
-- control at all — so the treasury payout loop could not be closed from the panel
-- (a capability reachable only via SQL is a defect by the parity rule).
--
-- The destination now lives on point_treasury, per network, alongside the other
-- admin-editable settlement settings. provider_keys metadata remains read as a
-- backwards-compatible fallback when this column is empty.

ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS payout_destination TEXT;
