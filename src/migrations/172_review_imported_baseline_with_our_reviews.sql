-- 172_review_imported_baseline_with_our_reviews.sql — capture the imported baseline for the
-- listings migration 171 deliberately skipped (item 1 reopened: moderation still destroyed the
-- imported reputation of a listing that already had an approved review of ours).
--
-- 171 added imported_rating/imported_review_count and backfilled every listing that carried a
-- rating/count with NO approved rows of ours behind it. Its update is guarded by
-- `NOT EXISTS (approved review of ours)`, so a listing that ALREADY had an approved review kept both
-- imported_* NULL — while still carrying its imported numbers in `businesses.rating` /
-- `businesses.review_count`. Measured on the live database 2026-10-16: 24 listings were in exactly
-- that state (Miyako Japanese Buffet 4.6/5238, Cucina Dania Beach 4.8/4090, M&M Property Management
-- 4.7/842, …), each with one approved review of ours.
--
-- For those the recompute in `set_review_status_owned` / `delete_review` uses
-- COALESCE(imported_review_count, 0) = 0 as its baseline, i.e. it REPLACES the listing's reputation
-- with our own rows only: measured 2026-10-16, approving one review on a disposable copy of that
-- exact shape left the listing at 5.0/2 instead of ~4.6/5239. Nothing re-imports those numbers — a
-- one-way loss of the listing's public reputation.
--
-- This captures the baseline for those rows: the listing's own count is whatever its stored count
-- exceeds our approved rows by, and its own average is the rating it displays today. Their visible
-- `rating`/`review_count` are NOT touched — after this file a listing still reads exactly what it
-- read before, until it is moderated, at which point the recompute adds instead of replacing.
--
-- Idempotent (guarded by `imported_rating IS NULL`), additive and display-neutral. `src/db.rs`
-- re-applies every file in src/migrations on each boot, so an operator-set baseline is never
-- overwritten. The runtime keeps the same guard for listings imported later:
-- `handlers::reviews::capture_imported_baseline`.

UPDATE public.businesses b
SET imported_rating = b.rating,
    imported_review_count = GREATEST(
        COALESCE(b.review_count, 0)
          - (SELECT COUNT(*) FROM public.reviews r
             WHERE r.business_id = b.id AND r.status = 'approved'),
        0)
WHERE b.imported_rating IS NULL
  AND b.imported_review_count IS NULL
  AND COALESCE(b.review_count, 0) > (
      SELECT COUNT(*) FROM public.reviews r
      WHERE r.business_id = b.id AND r.status = 'approved');
