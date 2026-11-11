-- W30 — SATELLITE TOWN APPROVAL (per directory).
--
-- Town pages (`/<city>/<town-slug>`) used to publish the moment a town had three listings:
-- no human decision, and a new town in the import data could put a page live (and into the
-- sitemap) with nobody having looked at it. Now every town is a ROW here:
--   * detected towns are inserted `approved = false` (draft) — they publish nothing;
--   * an admin approves a town one at a time, and only then may its page exist;
--   * the >= 3 listings floor still applies on top, so approval can never publish a thin page.
-- `directories.include_satellite_towns` is the per-city master switch (unchecking it unpublishes
-- every town of that city without losing the approvals).
--
-- The seed below approves every town that publishes a page TODAY (three or more active listings
-- of its own town), so this migration changes nothing about what is live for existing cities.

ALTER TABLE directories
    ADD COLUMN IF NOT EXISTS include_satellite_towns boolean NOT NULL DEFAULT true;

CREATE TABLE IF NOT EXISTS satellite_towns (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id  uuid NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    town_key      text NOT NULL,
    town_label    text NOT NULL,
    approved      boolean NOT NULL DEFAULT false,
    approved_at   timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (directory_id, town_key)
);

CREATE INDEX IF NOT EXISTS idx_satellite_towns_directory
    ON satellite_towns (directory_id, approved);

INSERT INTO satellite_towns (directory_id, town_key, town_label, approved, approved_at)
SELECT b.directory_id,
       LOWER(SPLIT_PART(TRIM(b.city), ',', 1)) AS town_key,
       MIN(TRIM(SPLIT_PART(TRIM(b.city), ',', 1))) AS town_label,
       true,
       now()
  FROM businesses b
 WHERE b.is_active = true
   AND b.city IS NOT NULL
   AND TRIM(b.city) <> ''
 GROUP BY b.directory_id, LOWER(SPLIT_PART(TRIM(b.city), ',', 1))
HAVING COUNT(*) >= 3
ON CONFLICT (directory_id, town_key) DO NOTHING;
