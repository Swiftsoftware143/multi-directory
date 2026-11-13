-- lastmod honesty for listing pages (kanban t_23332bb8).
--
-- Measured 2026-11-13: the description sweep filled 4,099 blank `businesses.description`
-- values at 00:05:55–00:06:24Z, so every live listing detail page gained a paragraph of real
-- copy — and the sitemap did not move for any of them. `<lastmod>` came from
-- `businesses.updated_at`, which the description writer never touched, so most of those URLs
-- still advertised 2026-07-18: about four months younger than the page they point at. A
-- crawler therefore had no reason to refetch copy that had genuinely changed.
--
-- `updated_at` cannot be the only source: it also moves for bookkeeping that changes nothing a
-- visitor sees (the monthly enrichment pass — see f03062c — provider/CRM ids, Stripe status).
-- This column records the one thing the sitemap actually means: the last time the copy rendered
-- on the listing page changed. Writers that change rendered copy stamp it in the same statement
-- that changes that copy (src/utils/business_copy.rs); the sitemap reads
-- GREATEST(updated_at, content_updated_at) so nothing regresses.
--
-- Nullable on purpose: NULL means "no recorded copy change since this column existed", and
-- GREATEST() ignores NULL, so those rows keep their honest `updated_at` date.

ALTER TABLE businesses ADD COLUMN IF NOT EXISTS content_updated_at timestamptz;

COMMENT ON COLUMN businesses.content_updated_at IS
  'Last time copy rendered on the public listing page changed (description and friends). Distinct from updated_at, which also moves for bookkeeping. Read by the sitemap as GREATEST(updated_at, content_updated_at).';

-- ONE deliberate backfill — documented, not mechanical (card t_23332bb8), and the only bulk
-- stamp this card makes. Scope is exact: a live row whose description the 2026-11-13T00:06Z
-- sweep wrote, recognised by shape alone — every paragraph compose() emits starts with the
-- listing's own name, and exactly one active listing (Pink Shimmering Maids, a real
-- owner-written paragraph) does not match that shape, so it is deliberately NOT stamped.
-- The instant is the sweep's own last recorded write (business_copy log, 00:06:24.181772Z);
-- no row claims a change later than it really had one.
UPDATE businesses
   SET content_updated_at = '2026-11-13T00:06:24.181772Z'::timestamptz
 WHERE content_updated_at IS NULL
   AND is_active
   AND coalesce(btrim(description), '') <> ''
   AND description LIKE name || ' %';
