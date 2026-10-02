-- Phase 3d: Per-user claim limit, highlights validation, and enhanced deal fields
-- Adds per_user_limit to deals table for free-tier per-user claim enforcement

-- 1. Per-user claim limit (nullable = unlimited, for free tier)
ALTER TABLE deals ADD COLUMN IF NOT EXISTS per_user_limit INTEGER;

-- 2. Ensure highlights jsonb column exists (used in Phase 3b, may have been added elsewhere)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'deals' AND column_name = 'highlights'
    ) THEN
        ALTER TABLE deals ADD COLUMN highlights JSONB DEFAULT '[]'::jsonb;
    END IF;
END $$;

-- 3. Add index for per-user claim lookups on deal_redemptions
CREATE INDEX IF NOT EXISTS idx_deal_redemptions_deal_visitor
    ON deal_redemptions(deal_id, visitor_id);
