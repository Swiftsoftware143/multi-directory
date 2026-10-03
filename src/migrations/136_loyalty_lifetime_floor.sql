-- B46: lifetime floor — the minimum LIFETIME currency a member must have EARNED before they may
-- redeem. Closes the sign-up-and-drain hole: a joining/one-off award can no longer be redeemed and
-- drained before the member has actually earned anything at the programme's own rate.
-- 0 = no floor (the default). Admin-configurable per programme (network- or directory-scoped) in
-- the loyalty editor, alongside earn rate / redemption cap / minimum redeem balance.
ALTER TABLE loyalty_programs
    ADD COLUMN IF NOT EXISTS min_lifetime_floor integer NOT NULL DEFAULT 0;
