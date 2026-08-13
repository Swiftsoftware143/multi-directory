-- 074_franchise_flag.sql
-- Additive migration (never subtractive).
--
-- PURPOSE: franchise / big-chain exclusion only. The supplier taxonomy is the
-- `business_type` field (source of truth), NOT a separate categories table.
--
-- Authoritative business_type values (per David):
--   local, supplier, distributor, wholesaler, farm, association, manufacture(r)
--
-- This migration:
--   1. Adds `manufacturer` + `chain` to the business_type CHECK (additive).
--   2. Adds `is_franchise` flag for franchise/big-chain exclusion (auto-flag on
--      publish via Google `types` + manual override).
--
-- NOTE: No supplier_categories table and no supplier_category_id column.
-- Supplier "categories" = business_type, per David's correction.

-- 1. Extend business_type (additive — keeps existing values).
ALTER TABLE businesses DROP CONSTRAINT IF EXISTS businesses_business_type_check;
ALTER TABLE businesses ADD CONSTRAINT businesses_business_type_check
  CHECK (business_type = ANY (ARRAY[
    'local', 'supplier', 'distributor', 'wholesaler', 'farm', 'association', 'manufacturer', 'chain'
  ]::text[]));

-- 2. Franchise/chain flag.
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS is_franchise BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX IF NOT EXISTS idx_businesses_is_franchise ON businesses (is_franchise)
  WHERE is_franchise = true;
