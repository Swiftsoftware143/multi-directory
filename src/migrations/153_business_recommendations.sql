-- 153_business_recommendations.sql — "Recommend a business" on a listing (card B90, Nextdoor-style).
--
-- A signed-in shopper publicly endorses a business; the directory shows a public "recommended by N
-- neighbours" count as social proof. This is Nextdoor's core social verb and is deliberately
-- distinct from the two existing customer->listing relationships: visitor_favorites ("Save") is a
-- private bookmark, and business_follows (152) is a private subscription. A recommendation is
-- public and carries a count.
--
-- Mirrors business_follows' FK semantics on purpose: business_id CASCADEs (a recommendation is
-- meaningless once its listing is gone), visitor_account_id CASCADEs, and directory_id is
-- denormalised + nullable ON DELETE SET NULL (a directory outlives its recommendations, and a
-- directory-less listing can still be recommended).
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

CREATE TABLE IF NOT EXISTS public.business_recommendations (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id        uuid NOT NULL REFERENCES public.businesses(id) ON DELETE CASCADE,
    visitor_account_id uuid NOT NULL REFERENCES public.visitor_accounts(id) ON DELETE CASCADE,
    directory_id       uuid REFERENCES public.directories(id) ON DELETE SET NULL,
    created_at         timestamptz NOT NULL DEFAULT now()
);

-- One recommendation per (customer, business); the toggle relies on this for idempotency.
CREATE UNIQUE INDEX IF NOT EXISTS business_recommendations_visitor_business_key
    ON public.business_recommendations (visitor_account_id, business_id);

CREATE INDEX IF NOT EXISTS idx_business_recommendations_business  ON public.business_recommendations (business_id);
CREATE INDEX IF NOT EXISTS idx_business_recommendations_visitor   ON public.business_recommendations (visitor_account_id);
CREATE INDEX IF NOT EXISTS idx_business_recommendations_directory ON public.business_recommendations (directory_id);
CREATE INDEX IF NOT EXISTS idx_business_recommendations_created   ON public.business_recommendations (created_at DESC);
