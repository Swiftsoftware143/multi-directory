-- Add api_config column for per-directory API keys (Google Places, Yelp, etc.)
ALTER TABLE directories ADD COLUMN IF NOT EXISTS api_config jsonb DEFAULT '{}'::jsonb;
