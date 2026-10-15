-- 171_business_imported_rating.sql — keep a listing's imported rating/count when it is moderated
-- (item 1 reopened: APPROVING a first review destroyed the listing's imported reputation).
--
-- A listing imported from public data carries its star rating and review count in
-- `businesses.rating` / `businesses.review_count` and has NO rows in `reviews` — 3,914 of 3,988
-- listings are in exactly that state (measured 2026-10-15). Two arms of review moderation
-- (`DELETE` of a never-approved review, and a reject/no-op status change) were already taught to
-- leave such a listing alone, but the APPROVE arm still recomputed the aggregates as
-- `AVG/COUNT(reviews WHERE status='approved')` — i.e. it REPLACED the imported numbers with our own
-- tiny set. Measured: `Pink Shimmering Maids` went 4.8/120 -> 5.0/1 on the owner approving their
-- first review, and nothing re-imports those numbers afterwards (a one-way loss).
--
-- The listing's own front-end attributes the imported number ("N reviews from public listings")
-- and the business-owner guide promises that approving a review *updates* the average and count —
-- i.e. incorporate, not replace. So the reconciled aggregate is
--   review_count = imported_count + our approved rows
--   rating       = (imported_rating*imported_count + SUM(our approved ratings)) / review_count
--
-- This file adds the two nullable columns that hold the imported baseline and backfills them for
-- every listing that currently has a rating/count with no approved rows of ours behind it. A
-- listing that already has approved reviews keeps imported_* NULL, so its display is unchanged.
--
-- Idempotent: the backfill is guarded by `imported_rating IS NULL`, so a re-run (src/db.rs
-- re-applies every file in src/migrations on each boot) never overwrites a captured baseline, and
-- an operator/resolver-set value is never clobbered. Additive and nullable — no existing
-- behaviour changes until a listing carrying an imported baseline is moderated.

ALTER TABLE public.businesses ADD COLUMN IF NOT EXISTS imported_rating double precision;
ALTER TABLE public.businesses ADD COLUMN IF NOT EXISTS imported_review_count integer;

UPDATE public.businesses b
SET imported_rating = b.rating,
    imported_review_count = b.review_count
WHERE b.imported_rating IS NULL
  AND b.rating IS NOT NULL
  AND COALESCE(b.review_count, 0) > 0
  AND NOT EXISTS (
      SELECT 1 FROM public.reviews r
      WHERE r.business_id = b.id AND r.status = 'approved'
  );
