-- 127_business_tables_link.sql — B75 ("Fix the business tables")
--
-- TWO business tables, ONE real record each — they were never linked, and that is
-- why the directory's "business count" disagreed with itself (4105 / 4000 / 4002 /
-- 3989 depending on which screen asked, kanban B75).
--
-- MEASURED on live, 2026-10-02 (audit /opt/swift/audits/t_b75_builds/):
--   * `businesses`           4004 rows — the AUTHORITATIVE global record
--                            (name, slug, city, claim, owner_id, verified, messaging,
--                             supplier_fields, SEO). status='active' and is_active=true
--                             on all 4004.
--   * `business_listings`    3989 rows — a CITY-PAGE PROJECTION (business_name,
--                            city_page_id, display_order, is_featured, is_editors_pick,
--                            category). It is LIVE: the visitor-facing city pages render
--                            their business cards from it (src/handlers/zaarhub_cities.rs,
--                            zaarhub_ssr.rs, zaarhub_analytics.rs).
--   * 3498 rows share a UUID across the two tables.
--   * 491 rows exist ONLY in business_listings. Classified:
--       - 490 are the SAME real business as an existing `businesses` row — exact
--         name + address match — under a DIFFERENT UUID (the Palm Bay seed wrote its own
--         id space). These are the duplicates.
--       - 1 is the test fixture 'Test Restaurant' (123 Main St) — orphaned test data.
--       - 0 genuine businesses are missing from `businesses`.
--
-- DECISION (recorded in docs/business-tables.md):
--   `businesses` is the single source of truth for a business record.
--   `business_listings` MUST STAY (city pages depend on it), so — per B75(c) — it is
--   DOCUMENTED here and LINKED to the authoritative row instead of being retired.
--   The canonical business count is:  SELECT count(*) FROM businesses WHERE is_active;
--   A count of `business_listings` is a count of city-page CARDS, not of businesses.
--
-- NON-DESTRUCTIVE: nothing is deleted, renamed or merged. This migration only adds a
-- nullable link column, backfills it where the join is unambiguous, and adds a read view.
-- Actual removal of the classified rows is a separate, reported step (B75 forbids
-- deleting before the 491 are classified and the classification is reported).
--
-- Idempotent: safe on a fresh install (the baseline creates business_listings without
-- this column) and safe to re-run on a live DB that already has it.

ALTER TABLE business_listings ADD COLUMN IF NOT EXISTS business_id uuid;

COMMENT ON TABLE business_listings IS
  'City-page presentation projection of `businesses` (kanban B75). One row per business '
  'card shown on a city page; carries city_page_id, display_order, is_featured, '
  'is_editors_pick, category. `businesses` is the AUTHORITATIVE global business record '
  '(name/slug/city/claim/owner/verified/messaging/suppliers/SEO); this table links to it '
  'via business_id. A count of this table is a count of city-page CARDS, NOT the '
  'directory''s business count — use SELECT count(*) FROM businesses WHERE is_active for that.';

COMMENT ON COLUMN business_listings.business_id IS
  'FK to businesses(id): the authoritative business record this city-page card presents '
  '(kanban B75). NULL only for rows not yet reconciled to a business record.';

-- (1) Rows that already share their UUID with `businesses`: that UUID IS the business id.
UPDATE business_listings bl
   SET business_id = bl.id
 WHERE bl.business_id IS NULL
   AND EXISTS (SELECT 1 FROM businesses b WHERE b.id = bl.id);

-- (2) Remaining rows: backfill by the only join the legacy data supports — an exact
--     name + address match. min(b.id) keeps the choice deterministic where a name+address
--     is (mistakenly) shared by two businesses rows (2 such groups on live, 2026-10-02).
UPDATE business_listings bl
   SET business_id = m.bid
  FROM (
        SELECT bl2.id AS lid, min(b.id::text)::uuid AS bid
          FROM business_listings bl2
          JOIN businesses b
            ON lower(btrim(b.name)) = lower(btrim(bl2.business_name))
           AND lower(btrim(coalesce(b.address, ''))) = lower(btrim(coalesce(bl2.address, '')))
         WHERE bl2.business_id IS NULL
         GROUP BY bl2.id
       ) m
 WHERE bl.id = m.lid;

ALTER TABLE business_listings DROP CONSTRAINT IF EXISTS business_listings_business_id_fkey;

ALTER TABLE business_listings
  ADD CONSTRAINT business_listings_business_id_fkey
  FOREIGN KEY (business_id) REFERENCES businesses(id) ON DELETE SET NULL NOT VALID;

ALTER TABLE business_listings VALIDATE CONSTRAINT business_listings_business_id_fkey;

CREATE INDEX IF NOT EXISTS idx_business_listings_business_id
    ON business_listings (business_id);

-- The canonical read surface: one row per authoritative business, with the number of
-- city pages that present it. Screens that want "how many businesses" count this view
-- (or `businesses` directly); screens that want "how many cards on this city page" still
-- count business_listings WHERE city_page_id = <the page>.
CREATE OR REPLACE VIEW v_businesses_canonical AS
SELECT b.id            AS business_id,
       b.name          AS business_name,
       b.slug,
       b.city,
       b.status,
       b.is_active,
       b.claimed,
       b.verified,
       (SELECT count(*)
          FROM business_listings bl
         WHERE bl.business_id = b.id) AS city_page_count
  FROM businesses b;

COMMENT ON VIEW v_businesses_canonical IS
  'B75 canonical business list: one row per `businesses` record (the source of truth), '
  'with city_page_count = how many city-page cards present it. Use this (or '
  'count(*) FROM businesses WHERE is_active) for the directory business count.';
