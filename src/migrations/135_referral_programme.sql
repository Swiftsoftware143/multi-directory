-- Card B48 — referral programme (per directory, admin-verified, native to Multi-Directory).
--
-- Model: one referral CODE per member (minted by feed::generate_referral_code), and a
-- referral ROW per signup that presents that code. That means several rows may carry the
-- same referral_code, so the generator's platform-wide uniqueness on the column has to go.
-- Codes stay collision-free because generation checks the table before inserting.
ALTER TABLE public.referrals DROP CONSTRAINT IF EXISTS referrals_referral_code_key;
