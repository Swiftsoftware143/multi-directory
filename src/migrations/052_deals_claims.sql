-- Deals & Coupons Phase 1: New deal_claims table and deal enhancements
-- Adds Groupon-style claim tracking with unique claim codes

-- Add new columns to existing deals table for Phase 1 discount system
ALTER TABLE deals ADD COLUMN IF NOT EXISTS discount_type VARCHAR(20) NOT NULL DEFAULT 'percentage';
ALTER TABLE deals ADD COLUMN IF NOT EXISTS discount_value DECIMAL(10,2);
ALTER TABLE deals ADD COLUMN IF NOT EXISTS max_claims INTEGER;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS claims_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS starts_at TIMESTAMPTZ;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT true;

-- Add check constraint for discount_type if not exists
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'deals_discount_type_check'
    ) THEN
        ALTER TABLE deals ADD CONSTRAINT deals_discount_type_check
            CHECK (discount_type IN ('percentage', 'fixed_amount'));
    END IF;
END $$;

-- Deal claims table: tracks who claimed each deal
CREATE TABLE IF NOT EXISTS deal_claims (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    deal_id UUID NOT NULL REFERENCES deals(id) ON DELETE CASCADE,
    visitor_name VARCHAR(255) NOT NULL,
    visitor_email VARCHAR(255) NOT NULL,
    visitor_phone VARCHAR(50),
    claim_code VARCHAR(20) NOT NULL UNIQUE,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    redeemed_at TIMESTAMPTZ,
    notes TEXT
);

CREATE INDEX IF NOT EXISTS idx_deal_claims_deal_id ON deal_claims(deal_id);
CREATE INDEX IF NOT EXISTS idx_deal_claims_email ON deal_claims(visitor_email);
CREATE INDEX IF NOT EXISTS idx_deal_claims_code ON deal_claims(claim_code);

-- New indexes for existing deals table
CREATE INDEX IF NOT EXISTS idx_deals_active ON deals(is_active);
CREATE INDEX IF NOT EXISTS idx_deals_business_id ON deals(business_id);
ALTER TABLE deals ADD COLUMN IF NOT EXISTS deal_price_numeric DECIMAL(10,2);

-- Set discount_value for existing deals from discount_percent
UPDATE deals SET discount_value = discount_percent::decimal WHERE discount_value IS NULL AND discount_percent IS NOT NULL;
