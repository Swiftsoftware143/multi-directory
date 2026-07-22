-- Migration 033: Neighborhood Feed support
-- Adds any missing indexes and a column for feed preferences on visitor_accounts

-- Add survey_answered_at to visitor_accounts for onboarding flow tracking
ALTER TABLE visitor_accounts ADD COLUMN IF NOT EXISTS survey_answered_at TIMESTAMPTZ;

-- Ensure businesses table has a description column that the feed template uses
-- (already has one in most schemas, but ensure it)
-- Also ensure images is accessible

-- Add an index on businesses category_id + directory_id for faster suggestion queries
CREATE INDEX IF NOT EXISTS idx_businesses_category_directory ON businesses(category_id, directory_id);

-- Index on survey_responses for visitor lookup
CREATE INDEX IF NOT EXISTS idx_survey_responses_visitor ON survey_responses(visitor_account_id);

INSERT INTO _migrations (filename) VALUES ('033_neighborhood_feed.sql');
