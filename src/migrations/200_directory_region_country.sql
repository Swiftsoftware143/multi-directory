-- Homepage long-term city nav — REGION + COUNTRY (work-queue line 52a: "every directory
-- stores REGION + STATE + COUNTRY from day one"). `state` already exists; this adds the two
-- missing geography levels so the future region -> state -> city drill-down never needs a
-- backfill. Both are free-text and admin-editable from the panel's directory editor, so a
-- directory can live anywhere in the world — never hardcode a region or country in code.
ALTER TABLE directories ADD COLUMN IF NOT EXISTS region text;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS country text;

-- Backfill the live Florida cities (idempotent, and a no-op on a fresh install that has none).
UPDATE directories
   SET region = 'Southeast',
       country = 'United States'
 WHERE COALESCE(NULLIF(btrim(state), ''), '') = 'FL'
   AND (region IS NULL OR btrim(region) = '')
   AND (country IS NULL OR btrim(country) = '');
