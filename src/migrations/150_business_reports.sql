-- 150_business_reports.sql — "Report a problem" on a business listing (card B90 cross-cutting).
--
-- David's competitive-parity mandate (2026-10-01): favourites/saved, share a listing, report a
-- problem, follow a business … are APPROVED build items. This migration adds the storage for the
-- reporting half: a visitor flags a listing as closed / wrong info / spam / offensive / duplicate /
-- other, and a non-technical directory admin works the queue from the admin panel (no SQL, no shell
-- — see card B84, the sellable-standard bar).
--
-- directory_id is denormalised from the business so the moderation queue can be scoped to one
-- directory without a join, and is ON DELETE SET NULL (a directory outlives its reports; deleting a
-- directory must never delete its businesses' reports). business_id cascades, because a report is
-- meaningless once its listing is gone.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

CREATE TABLE IF NOT EXISTS public.business_reports (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id     uuid NOT NULL REFERENCES public.businesses(id) ON DELETE CASCADE,
    directory_id    uuid REFERENCES public.directories(id) ON DELETE SET NULL,
    reason          text NOT NULL,
    details         text,
    reporter_name   text,
    reporter_email  text,
    status          text NOT NULL DEFAULT 'pending',
    resolution_note text,
    resolved_at     timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now()
);

-- Closed vocabularies so the admin panel can render filters without guessing the values.
ALTER TABLE public.business_reports DROP CONSTRAINT IF EXISTS business_reports_reason_check;
ALTER TABLE public.business_reports
    ADD CONSTRAINT business_reports_reason_check
    CHECK (reason IN ('closed', 'wrong_info', 'spam', 'offensive', 'duplicate', 'other'));

ALTER TABLE public.business_reports DROP CONSTRAINT IF EXISTS business_reports_status_check;
ALTER TABLE public.business_reports
    ADD CONSTRAINT business_reports_status_check
    CHECK (status IN ('pending', 'reviewed', 'resolved', 'dismissed'));

CREATE INDEX IF NOT EXISTS idx_business_reports_status    ON public.business_reports (status);
CREATE INDEX IF NOT EXISTS idx_business_reports_directory ON public.business_reports (directory_id);
CREATE INDEX IF NOT EXISTS idx_business_reports_business  ON public.business_reports (business_id);
CREATE INDEX IF NOT EXISTS idx_business_reports_created   ON public.business_reports (created_at DESC);
