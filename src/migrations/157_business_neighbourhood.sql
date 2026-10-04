-- 157_business_neighbourhood.sql — "Neighbourhood" granularity for a business (card B90,
-- Nextdoor-style competitive parity).
--
-- David's competitive-parity mandate (2026-10-01) approves "suburb/neighbourhood granularity":
-- Nextdoor's defining browse surface is the neighbourhood BELOW the city, and ZaarHub is
-- city-scoped (directories.city) with no way to say where within a city a business sits. This
-- adds one operator-set free-text label per business (e.g. "Palm Bay West", "Downtown Melbourne")
-- so the public surfaces can group and filter a city page by neighbourhood.
--
-- Admin-settable from the Businesses tab of the admin panel (card B84: the buyer runs this
-- without SQL). Nullable and plain text because a directory may not use neighbourhoods at all —
-- the public surfaces simply omit the group/filter when no business has one.
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

ALTER TABLE public.businesses ADD COLUMN IF NOT EXISTS neighbourhood text;

CREATE INDEX IF NOT EXISTS idx_businesses_directory_neighbourhood
    ON public.businesses (directory_id, neighbourhood)
    WHERE neighbourhood IS NOT NULL;
