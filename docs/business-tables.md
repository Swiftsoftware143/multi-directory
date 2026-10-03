# The two business tables — authoritative vs projection (B75)

**Status:** decided 2026-10-02. Migration: `src/migrations/127_business_tables_link.sql`.
**Evidence:** `/opt/swift/audits/t_b75_builds/` (probe1–probe4).

## The decision in one line

> `businesses` is the **authoritative** business record. `business_listings` is a
> **city-page projection** of it and must stay. The canonical business count is
> `SELECT count(*) FROM businesses WHERE is_active`.

## What each table is for

| | `businesses` | `business_listings` |
|---|---|---|
| Role | The business record (source of truth) | A card shown on ONE city page |
| Key shape | `id`, `name`, `slug`, `city`, `status`, `is_active`, `claimed`, `owner_id`, `verified`, `supplier_fields`, `search_vector` | `id`, `business_name`, `city_page_id`, `category`, `display_order`, `is_featured`, `is_editors_pick`, `deal_text` |
| Read by | business pages, claims, messaging, supplier portal, SEO, admin | visitor-facing city pages (`zaarhub_cities.rs`, `zaarhub_ssr.rs`, `zaarhub_analytics.rs`) |
| Count on 2026-10-02 | **4004** (all `status='active'`, all `is_active=true`) | **3989** cards across 10 city pages |

They are **not** mirrors. `business_listings` has no `name` column and a different UUID
space; it exists so a city page can rank/feature a card (`display_order`, `is_featured`,
`is_editors_pick`) without touching the global record.

## Why the count disagreed

The two tables had **no key linking them**. The count a screen reported depended on which
table it read:

* admin / homepage stats / most handlers → `businesses` → **4004**
* ZaarHub city pages and `zaarhub_analytics` → `business_listings` → **3989**
* the Palm Bay seed had written 490 businesses under its **own** UUID, so a naive union
  double-counted them → the higher figures (4105 / 4000).

## Classification of the 491 listings-only rows (B75(b))

| Bucket | Count | Action |
|---|---|---|
| Same real business as an existing `businesses` row (exact name + address), different UUID | **490** | Linked via `business_listings.business_id` (migration 127). **Kept** — these 490 ARE Palm Bay's city-page cards; deleting them would empty the city page. The defect was the missing link, not extra rows. |
| Orphaned test data — `Test Restaurant`, 123 Main St | **1** | **Deleted 2026-10-02.** Unreferenced (`claim_offers.listing_id` is the only FK into `business_listings`; 0 rows pointed at it). It was a live public card on the Palm Bay page. |
| Genuine businesses missing from `businesses` | **0** | — |

The other direction: 506 `businesses` rows have no city-page card — legitimate
(businesses not attached to any of the 10 city pages), not an error.

## What migration 127 does (non-destructive)

1. adds nullable `business_listings.business_id uuid`, backfilled for 3988 of the 3989
   cards (3498 by shared UUID, 490 by exact name + address) — the last card was the
   `Test Restaurant` fixture, since deleted;
2. adds FK `business_listings_business_id_fkey → businesses(id) ON DELETE SET NULL`
   (added `NOT VALID`, then `VALIDATE`d) + an index;
3. adds `COMMENT`s on the table and the column recording the roles above;
4. adds view `v_businesses_canonical` — one row per `businesses` record plus
   `city_page_count`, the canonical read surface.

Nothing is renamed, merged or deleted.

## The one number

```
SELECT count(*) FROM businesses WHERE is_active;   -- canonical business count
```

`md-health.sh` prints it and asserts that **every** `business_listings` row is linked to a
`businesses` row. City-page card counts
(`business_listings WHERE city_page_id = …`) are a **different, legitimate** number and
must never be presented as "the directory business count".

## Follow-ups (not done this shift)

* Point remaining screens that surface a *directory-wide* business count at the canonical
  source: `frontend/index.html` (`totalBiz`, the `cities.reduce(... business_count)` fallback)
  and `frontend/admin-panel.html` (`totalBusinesses` sum) still derive from per-city
  `business_count`, which is a city-page card count.
* The 490 Palm Bay cards and their `businesses` records still carry two UUIDs for one real
  business (URLs on the city page use the listing id). Migration 127 links them, so a future
  id-space merge is mechanical — but the merge itself is a separate, carded change.
