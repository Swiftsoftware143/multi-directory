-- 141_claim_email_verification.sql — card B76: automated claim verification.
--
-- The business-claim flow ALREADY auto-accepts a claim when the claimant's email domain matches
-- the listing's website (or the claimant's address equals the listing's email address). What it
-- never did was the SECOND step for everyone else: a non-matching claim told the owner "our team
-- will review your claim within 24 hours" and an auto-accepted one told them "check your email for
-- login credentials" — while no email was sent at all and there was no way for a claimant to prove
-- ownership by email.
--
-- This puts the two-step token ON the claim itself (one claim, one live token — no new table) and
-- the bookkeeping around it. Every column is NULLABLE, so every existing row and every real
-- customer row is byte-identical to before this file existed.
ALTER TABLE public.claimed_businesses
    ADD COLUMN IF NOT EXISTS verification_token text,
    ADD COLUMN IF NOT EXISTS verification_sent_at timestamp with time zone,
    ADD COLUMN IF NOT EXISTS email_verified_at timestamp with time zone;

-- The confirm endpoint looks a claim up BY token, so a token maps to exactly one claim.
-- Partial: claims that never got a token (auto-accepted before this file, or seeded rows) are
-- not constrained and keep a NULL token.
CREATE UNIQUE INDEX IF NOT EXISTS claimed_businesses_verification_token_key
    ON public.claimed_businesses (verification_token)
    WHERE verification_token IS NOT NULL;
