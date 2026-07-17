-- Migration 017: Monetization Templates & External Payment Links
-- Adds columns for external checkout integration (Mint Bird / Groovesell)
-- and links between directory_tiers and plan_tiers

-- 1. plan_sales_page_url on plan_tiers — external checkout link (Mint Bird / Groovesell)
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS plan_sales_page_url TEXT;

-- 2. external_plan_id on directory_tiers — external payment system plan ID
ALTER TABLE directory_tiers ADD COLUMN IF NOT EXISTS external_plan_id TEXT;

-- 3. external_checkout_url on directory_tiers — override checkout URL per subscription
ALTER TABLE directory_tiers ADD COLUMN IF NOT EXISTS external_checkout_url TEXT;

-- 4. plan_tier_id FK to plan_tiers on directory_tiers
ALTER TABLE directory_tiers ADD COLUMN IF NOT EXISTS plan_tier_id UUID REFERENCES plan_tiers(id) ON DELETE SET NULL;

-- 5. external_payment_ref on sponsored_listings — external payment reference
ALTER TABLE sponsored_listings ADD COLUMN IF NOT EXISTS external_payment_ref TEXT;

-- 6. external_payment_ref on ad_zones — external payment reference
ALTER TABLE ad_zones ADD COLUMN IF NOT EXISTS external_payment_ref TEXT;

-- 7. external_payment_ref on business_subscriptions — external payment reference
ALTER TABLE business_subscriptions ADD COLUMN IF NOT EXISTS external_payment_ref TEXT;
