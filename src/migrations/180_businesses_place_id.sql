-- W29 — ONE BUSINESS = ONE RECORD (2026-11-11, builds)
--
-- Identity for a business used to be re-derived from a name slug on every entry
-- path. Two different businesses that share a name collapsed into one record, and
-- the same business re-imported under a slightly different name became two.
-- Google's place_id is the durable identity: it never changes for a place.
--
-- Unique per directory (a business may legitimately be listed in two cities —
-- those are two appearances backed by two records in different directories).
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS place_id text;

CREATE UNIQUE INDEX IF NOT EXISTS businesses_directory_place_id_key
    ON businesses (directory_id, place_id)
    WHERE place_id IS NOT NULL AND place_id <> '';

COMMENT ON COLUMN businesses.place_id IS
    'Google Places place_id — durable identity of this business; unique per directory (W29).';
