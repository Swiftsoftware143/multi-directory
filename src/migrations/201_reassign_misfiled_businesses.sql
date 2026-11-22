-- 201_reassign_misfiled_businesses.sql
--
-- Work-queue line 41 (LISTINGS): "move every mis-filed business to its
-- CLOSEST satellite town (by coordinates); the hub's own listings sort first,
-- reassigned ones at the BOTTOM of the list."
--
-- The sort half is already live (local_priority = 0 sinks to the bottom of
-- every listing query; the city hub renders its own town first). This file
-- does the data half: an ACTIVE business whose coordinates sit more than
-- 2 x LOCAL_AREA_KM (80 km) from its OWN directory centre was pulled in by an
-- over-broad import and is re-filed under the directory whose centre is
-- closest to it. It is then marked local_priority = 0 so it sorts LAST there.
--
-- Measured live (2026-11-21): 14 active businesses — Palm Beach / Palm Beach
-- Gardens / Palm City, filed under palm-coast and palm-bay but 100-330 km from
-- those centres — reassign to boca-raton (38-90 km away, their closest city).
--
-- Idempotent: after the move, a business's closest directory IS its own, so
-- the WHERE guard (closest <> own) never re-selects it. Newly created town
-- rows use ON CONFLICT (directory_id, town_key) DO NOTHING.

BEGIN;

-- 1. Re-file clearly mis-filed ACTIVE businesses under their closest directory.
WITH mis AS (
    SELECT
        b.id,
        b.directory_id,
        (
            SELECT d2.id
            FROM directories d2
            JOIN city_pages cp2 ON cp2.city_slug = d2.slug
            WHERE d2.id <> b.directory_id
            ORDER BY (
                6371 * acos(
                    cos(radians(cp2.latitude)) * cos(radians(b.latitude))
                    * cos(radians(b.longitude) - radians(cp2.longitude))
                    + sin(radians(cp2.latitude)) * sin(radians(b.latitude))
                )
            ) ASC
            LIMIT 1
        ) AS closest_dir_id
    FROM businesses b
    JOIN directories d ON d.id = b.directory_id
    JOIN city_pages cp ON cp.city_slug = d.slug
    WHERE b.latitude IS NOT NULL
      AND b.longitude IS NOT NULL
      AND b.is_active = true
      AND (
          6371 * acos(
              cos(radians(cp.latitude)) * cos(radians(b.latitude))
              * cos(radians(b.longitude) - radians(cp.longitude))
              + sin(radians(cp.latitude)) * sin(radians(b.latitude))
          )
      ) > 80
)
UPDATE businesses b
SET directory_id = m.closest_dir_id,
    local_priority = 0
FROM mis m
WHERE b.id = m.id
  AND m.closest_dir_id IS NOT NULL
  AND m.closest_dir_id <> m.directory_id;

-- 2. Give each re-filed business a town in its new directory so it still
--    renders on the new city's page (a business whose city matches no approved
--    town publishes nothing — see local_town_entries).
INSERT INTO satellite_towns (directory_id, town_key, town_label, approved, approved_at)
SELECT
    b.directory_id,
    lower(trim(b.city)),
    trim(b.city),
    true,
    now()
FROM businesses b
WHERE b.local_priority = 0
  AND b.directory_id IS NOT NULL
  AND b.city IS NOT NULL
  AND trim(b.city) <> ''
ON CONFLICT (directory_id, town_key) DO NOTHING;

COMMIT;
