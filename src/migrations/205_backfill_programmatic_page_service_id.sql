-- 205_backfill_programmatic_page_service_id.sql
--
-- Work-queue line 74/80 (PROGRAMMATIC PAGES rebuild): the renderer
-- (`render_programmatic_page`) is now data-driven — guidance, cost, FAQ,
-- business list and the category link all key off the page's `service_id`
-- (`LEFT JOIN directory_services`). The 5,376 `scheduled-trap-door` pages
-- carry a service_id and render ~350 words; the 320 `default` "service x
-- city" pages (e.g. `electrician-palm`, `accountant-palm`) have
-- `service_id = NULL`, so they skip every enrichment and render only the
-- thin stored blob (~125 words).
--
-- This file backfills that missing join key. The page title/h1 is always
-- "{Service name} in {City}" (e.g. "Accountant in Palm Bay"), so the
-- service is recovered by matching the h1 against `directory_services.name`
-- within the SAME directory. Measured live (2026-11-22): exactly 320 pages,
-- one unambiguous match each (0 unmatched, 0 multi-match per directory+slug),
-- so the join is safe.
--
-- Idempotent: only NULL-service_id `default` pages are touched; a re-run
-- matches nothing new. `location_id` is left alone — it is NULL on all 5,696
-- published pages and the renderer already falls back to the directory city.

BEGIN;

UPDATE programmatic_pages pp
SET service_id = ds.id,
    updated_at = NOW()
FROM directory_services ds
WHERE pp.service_id IS NULL
  AND pp.template_name = 'default'
  AND pp.status = 'published'
  AND ds.directory_id = pp.directory_id
  AND (pp.h1 ILIKE (ds.name || ' in %') OR pp.title ILIKE (ds.name || ' in %'));

COMMIT;
