-- Contact Intelligence Pipeline: add enriched_at to businesses
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS enriched_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_businesses_enriched_at ON businesses(enriched_at);
