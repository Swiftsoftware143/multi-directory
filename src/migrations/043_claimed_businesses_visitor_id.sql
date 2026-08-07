-- Migration 043: Add visitor_account_id to claimed_businesses for supplier portal auto-claim
-- Suppliers register via visitor_accounts, so we need a nullable visitor_account_id to link them
-- alongside the existing user_id (which references the users table for business owners)

ALTER TABLE claimed_businesses
  ADD COLUMN IF NOT EXISTS visitor_account_id UUID;

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_visitor
  ON claimed_businesses (visitor_account_id)
  WHERE visitor_account_id IS NOT NULL;

-- Add comment to document the column
COMMENT ON COLUMN claimed_businesses.visitor_account_id IS
  'Links to visitor_accounts.id for supplier portal registrations. Mutually exclusive with user_id in practice.';
