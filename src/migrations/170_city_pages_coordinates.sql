-- 170_city_pages_coordinates.sql — a city's own centre (item 18 follow-on: the Populate panel's
-- Radius control could never engage).
--
-- `frontend/zaarhub-admin.html`'s Populate tab resolves the picked city to a lat/lng and sends it
-- (with the operator's Radius, in metres) to GET /api/v1/zaarhub/admin/places/search as a Google
-- Places geo bias. Two gaps made that inert:
--   * the handler ignored lat/lng/radius entirely (fixed in src/handlers/zaarhub_admin.rs), and
--   * `GET /api/v1/zaarhub/cities` returned no coordinates for ANY city — `city_pages` had no
--     latitude/longitude column at all, so the panel's `popCityLatLng` map was empty for all 10
--     cities and the panel never sent the trio in the first place.
--
-- This adds the two columns and seeds the ten live city pages with their real centres, resolved
-- through the app's OWN Google Places lookup (GET /zaarhub/admin/places/search, 2026-10-15) —
-- the coordinate is the provider's answer for "<City>, FL, USA", not a hand-typed guess.
--
-- Nullable and additive; `is_active` and every existing page are untouched. The backfill is
-- guarded by `latitude IS NULL` so an operator-set (or later re-resolved) value is never
-- clobbered by a re-run — src/db.rs applies every file in src/migrations on each boot, so this
-- file must stay idempotent.

ALTER TABLE public.city_pages ADD COLUMN IF NOT EXISTS latitude double precision;
ALTER TABLE public.city_pages ADD COLUMN IF NOT EXISTS longitude double precision;

UPDATE public.city_pages cp
SET latitude = c.lat::double precision,
    longitude = c.lng::double precision,
    updated_at = now()
FROM (VALUES
    ('palm-bay',      28.0344621,  -80.5886646),
    ('st-cloud',      28.2488949,  -81.2812895),
    ('winter-garden', 28.5652787,  -81.5861847),
    ('st-petersburg', 27.7671271,  -82.6384451),
    ('apopka',        28.6934076,  -81.5322149),
    ('lake-nona',     28.3765852,  -81.2585677),
    ('hollywood',     26.0098511,  -80.1591420),
    ('boca-raton',    26.3683064,  -80.1289321),
    ('pompano-beach', 26.2305913,  -80.1251021),
    ('palm-coast',    29.5181443,  -81.2243162)
) AS c(slug, lat, lng)
WHERE cp.city_slug = c.slug
  AND cp.latitude IS NULL;
