ALTER TABLE business_listings ADD COLUMN IF NOT EXISTS is_editors_pick BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE business_listings ADD COLUMN IF NOT EXISTS editors_pick_note TEXT;
CREATE INDEX IF NOT EXISTS idx_business_listings_editors_pick ON business_listings (is_editors_pick) WHERE is_editors_pick = true;
