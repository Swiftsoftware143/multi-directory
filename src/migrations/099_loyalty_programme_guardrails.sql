-- 099: loyalty programme guardrails — the knobs that make a loyalty currency behave like money.
--
-- 094 made the *earning* rate explicit (points_per_redemption) but a programme still had no way to
-- say what its currency is WORTH, how much of a bill it may pay, or what a member must hold before
-- spending it. Multi-Directory is the platform ("Shopify for directories"): these are per-programme
-- settings an admin edits in the ⭐ Loyalty Programmes card, and a programme may be scoped to one
-- directory OR a whole network (ZaarHub runs ONE network-wide programme across ten cities).
--
-- Every column carries a default, so existing rows stay valid and behaviour is unchanged until an
-- admin edits a setting. 100 currency units = $1 across the platform, so min_redeem_balance 100 = $1.

ALTER TABLE loyalty_programs
    ADD COLUMN IF NOT EXISTS earn_rate double precision NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS redemption_cap_pct integer NOT NULL DEFAULT 10,
    ADD COLUMN IF NOT EXISTS min_redeem_balance integer NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS exclude_free_items boolean NOT NULL DEFAULT true;

COMMENT ON COLUMN loyalty_programs.earn_rate IS
    'Currency units credited per $1 of earnable spend. 0 = earning disabled. Default 1.';
COMMENT ON COLUMN loyalty_programs.redemption_cap_pct IS
    'Maximum percentage of a bill that a member may settle with the programme currency (0-100). Default 10.';
COMMENT ON COLUMN loyalty_programs.min_redeem_balance IS
    'Balance a member must hold before they may redeem. 100 units = $1, so the default 100 = $1.';
COMMENT ON COLUMN loyalty_programs.exclude_free_items IS
    'When true, free or fully-discounted items earn no currency. Default true.';
