-- 151_business_questions.sql — Local Q&A on a business listing (card B90, Nextdoor-style).
--
-- David's competitive-parity mandate (2026-10-01) approves a "local Q&A" surface: a customer
-- asks a business (or the community) a question and the directory shows the answered threads on
-- the listing. Until now a question could only reach a business through the private message form
-- and never surfaced publicly, so the same question was asked over and over — the exact gap a
-- Nextdoor-style directory closes.
--
-- A question is published immediately (a question is rarely abusive) and a moderator can hide or
-- delete it, or the directory admin answers it, from the ZaarHub admin panel (no SQL, no shell —
-- the sellable-standard bar of card B84). directory_id is denormalised from the business so the
-- moderation queue can be directory-scoped without a join; it is ON DELETE SET NULL (a directory
-- outlives its questions) while business_id cascades (a question is meaningless without its
-- listing). asker_email is stored for follow-up only and is NEVER returned by the public list.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

CREATE TABLE IF NOT EXISTS public.business_questions (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id  uuid NOT NULL REFERENCES public.businesses(id) ON DELETE CASCADE,
    directory_id uuid REFERENCES public.directories(id) ON DELETE SET NULL,
    asker_name   text,
    asker_email  text,
    question     text NOT NULL,
    answer       text,
    answered_at  timestamptz,
    status       text NOT NULL DEFAULT 'published',
    created_at   timestamptz NOT NULL DEFAULT now()
);

-- Closed vocabulary so the admin panel can render a filter without guessing the values.
ALTER TABLE public.business_questions DROP CONSTRAINT IF EXISTS business_questions_status_check;
ALTER TABLE public.business_questions
    ADD CONSTRAINT business_questions_status_check
    CHECK (status IN ('published', 'hidden'));

CREATE INDEX IF NOT EXISTS idx_business_questions_business  ON public.business_questions (business_id);
CREATE INDEX IF NOT EXISTS idx_business_questions_directory ON public.business_questions (directory_id);
CREATE INDEX IF NOT EXISTS idx_business_questions_status    ON public.business_questions (status);
CREATE INDEX IF NOT EXISTS idx_business_questions_created   ON public.business_questions (created_at DESC);
