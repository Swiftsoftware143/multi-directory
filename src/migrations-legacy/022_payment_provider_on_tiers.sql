-- Add payment_provider column to plan_tiers
-- Allows each plan tier to specify which payment processor to use
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS payment_provider VARCHAR(64);
