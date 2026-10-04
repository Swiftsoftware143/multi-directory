-- 152_business_follows.sql — "Follow a business" on a listing (card B90, Nextdoor-style).
--
-- David's competitive-parity mandate (2026-10-01) approves a "follow a business" surface: a
-- signed-in customer keeps an eye on a listing without adding it to their saved places, and the
-- directory shows a public follower count as a social-proof signal. Until now the only customer
-- relationship to a listing was favourites (visitor_favorites) — a private bookmark with no
-- follow semantics and no public count.
--
-- directory_id is denormalised from the business so a feed/notification job can scope by
-- directory without a join, and is ON DELETE SET NULL (a directory outlives its follows; deleting
-- a directory must never delete its businesses' followers). business_id cascades, because a
-- follow is meaningless once its listing is gone. directory_id is deliberately NULLABLE here:
-- unlike visitor_favorites (whose directory_id is NOT NULL), a directory-less listing can still
-- be followed.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

CREATE TABLE IF NOT EXISTS public.business_follows (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id        uuid NOT NULL REFERENCES public.businesses(id) ON DELETE CASCADE,
    visitor_account_id uuid NOT NULL REFERENCES public.visitor_accounts(id) ON DELETE CASCADE,
    directory_id       uuid REFERENCES public.directories(id) ON DELETE SET NULL,
    created_at         timestamptz NOT NULL DEFAULT now()
);

-- One follow per (customer, business); the toggle relies on this for idempotency.
CREATE UNIQUE INDEX IF NOT EXISTS business_follows_visitor_business_key
    ON public.business_follows (visitor_account_id, business_id);

CREATE INDEX IF NOT EXISTS idx_business_follows_business  ON public.business_follows (business_id);
CREATE INDEX IF NOT EXISTS idx_business_follows_visitor   ON public.business_follows (visitor_account_id);
CREATE INDEX IF NOT EXISTS idx_business_follows_directory ON public.business_follows (directory_id);
CREATE INDEX IF NOT EXISTS idx_business_follows_created   ON public.business_follows (created_at DESC);
