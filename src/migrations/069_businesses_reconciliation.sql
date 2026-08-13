-- 069_businesses_reconciliation: add columns the code reads/writes on `businesses`.
-- Follow-up to 068 (which was already applied before these ALTERs existed).

ALTER TABLE businesses ADD COLUMN IF NOT EXISTS directory_id UUID;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS category_id UUID;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS business_type VARCHAR(100);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS city VARCHAR(100);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS state VARCHAR(100);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS zip VARCHAR(20);
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS latitude DOUBLE PRECISION;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS longitude DOUBLE PRECISION;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS images JSONB;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS search_vector TSVECTOR;

CREATE INDEX IF NOT EXISTS idx_businesses_directory ON businesses(directory_id) WHERE directory_id IS NOT NULL;

-- Backfill: copy existing scalar city/geo fields into the new-named columns so
-- reminders and list views aren't empty for pre-existing rows.
UPDATE businesses
   SET directory_id = (SELECT d.id FROM directories d WHERE d.slug = businesses.city_slug LIMIT 1)
 WHERE directory_id IS NULL;
UPDATE businesses SET city = city_slug WHERE city IS NULL AND city_slug IS NOT NULL;
UPDATE businesses SET latitude = lat WHERE latitude IS NULL AND lat IS NOT NULL;
UPDATE businesses SET longitude = lng WHERE longitude IS NULL AND lng IS NOT NULL;
