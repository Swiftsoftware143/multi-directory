-- 094: points_per_redemption — the rate at which redeeming a deal earns loyalty points.
--
-- loyalty_programs already carried points_per_visit / points_per_checkin, but nothing credited a
-- member when they redeemed a deal: use_redemption only flipped status='used', so a customer could
-- redeem repeatedly and their wallet stayed at zero forever. This column makes the rate explicit and
-- admin-editable. 0 (the default) means "disabled" — behaviour is unchanged until an admin sets it.

ALTER TABLE loyalty_programs
    ADD COLUMN IF NOT EXISTS points_per_redemption integer NOT NULL DEFAULT 0;

COMMENT ON COLUMN loyalty_programs.points_per_redemption IS
    'Loyalty points credited to a member when they redeem a deal for this programme. 0 = disabled.';

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_directory_active
    ON loyalty_programs(directory_id) WHERE is_active = true;
