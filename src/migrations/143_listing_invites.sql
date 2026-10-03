-- 143_listing_invites.sql — card B82: supplier onboarding invite link.
--
-- A draft/prospect record (card B81) can be handed to the supplier who actually owns it: the
-- operator generates a ONE-TIME link bound to THAT business, the supplier opens it and completes
-- the listing. Completing ADOPTS the existing record (updates it in place) instead of inserting a
-- second business — so "prospect -> claimed supplier" is ONE row, never a duplicate.
--
-- One table; the token is the only credential (like the claim-verification token in migration 141),
-- so a link works from a mail client with no session. Every column except the FK and token is
-- NULLABLE, so no existing row is touched by this file.

CREATE TABLE IF NOT EXISTS public.listing_invites (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id uuid NOT NULL REFERENCES public.businesses(id) ON DELETE CASCADE,
    token text NOT NULL,
    email text,
    created_by uuid,
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    expires_at timestamp with time zone,
    used_at timestamp with time zone,
    used_by_visitor_id uuid,
    probe_harness text
);

-- The public complete page looks an invite up BY token, so one token maps to exactly one invite.
CREATE UNIQUE INDEX IF NOT EXISTS listing_invites_token_key
    ON public.listing_invites (token);

CREATE INDEX IF NOT EXISTS listing_invites_business_idx
    ON public.listing_invites (business_id);
