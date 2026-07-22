-- Stage 5: Referral System + Account Linking for SSO Role Switcher

-- 1. Account Links for SSO Role Switcher
CREATE TABLE IF NOT EXISTS account_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL,
    visitor_account_id UUID REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(email, visitor_account_id),
    UNIQUE(email, user_id)
);

CREATE INDEX IF NOT EXISTS idx_account_links_email ON account_links(email);

-- 2. Referrals table
CREATE TABLE IF NOT EXISTS referrals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    referrer_type TEXT NOT NULL,
    referrer_id UUID NOT NULL,
    referrer_email TEXT,
    referee_type TEXT NOT NULL,
    referee_id UUID,
    referee_email TEXT,
    referee_name TEXT,
    referral_code TEXT NOT NULL UNIQUE,
    direction TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    zaarcash_earned INTEGER DEFAULT 0,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_referrals_referrer ON referrals(referrer_id, referrer_type);
CREATE INDEX IF NOT EXISTS idx_referrals_code ON referrals(referral_code);
CREATE INDEX IF NOT EXISTS idx_referrals_status ON referrals(status);

-- Trigger to auto-update updated_at on referrals
CREATE OR REPLACE FUNCTION update_referrals_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_referrals_updated_at ON referrals;
CREATE TRIGGER trg_referrals_updated_at
    BEFORE UPDATE ON referrals
    FOR EACH ROW
    EXECUTE FUNCTION update_referrals_updated_at();

INSERT INTO _migrations (filename) VALUES ('035_referrals.sql');
