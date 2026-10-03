-- 135_loyalty_positioning.sql — card B93: the VIABLE-NOW parts of the loyalty positioning
-- (hero pill, 3-step process cards, earn-vs-redeem card, city-scoped hero, honest impact ticker).
--
-- Extends the B92 `loyalty_messaging` model (see 134_loyalty_messaging.sql) with the rest of the
-- positioning surface, every field admin-editable per NETWORK and per standalone DIRECTORY with the
-- same resolution rule (directory -> network -> built-in defaults).
--
-- DELIBERATELY NOT HERE (gated until the mechanism exists — a claim the product cannot deliver is a
-- defect, not copy): "earn on every completed job" (needs purchase/completion verification, B47),
-- "verified local pros" (verification/licence badges are approved but unbuilt) and
-- "book vetted service providers" (booking is only partially present). The `show_earn_claims` flag
-- from 134 stays the switch the back end uses to keep the served wording honest.
--
-- The live impact ticker is REAL: `ticker_threshold` is the minimum value (in local dollars
-- reinvested) below which the whole ticker hides itself — a ticker reading $0 does more damage than
-- no ticker, so the honest default is hidden-until-genuine. Numbers are never mocked or inflated.
--
-- Idempotent: ADD COLUMN IF NOT EXISTS, so a restart re-applies cleanly and never rewrites data.
ALTER TABLE public.loyalty_messaging
    ADD COLUMN IF NOT EXISTS pill_text text,
    ADD COLUMN IF NOT EXISTS steps jsonb,
    ADD COLUMN IF NOT EXISTS earn_card_title text,
    ADD COLUMN IF NOT EXISTS earn_card_body text,
    ADD COLUMN IF NOT EXISTS redeem_card_title text,
    ADD COLUMN IF NOT EXISTS redeem_card_body text,
    ADD COLUMN IF NOT EXISTS ticker_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN IF NOT EXISTS ticker_threshold double precision NOT NULL DEFAULT 100.0;

COMMENT ON COLUMN public.loyalty_messaging.pill_text IS
    'Hero pill/badge text. Supports {city} and {currency} tokens (interpolated server-side).';
COMMENT ON COLUMN public.loyalty_messaging.steps IS
    'Up to three {"title","body"} process cards; {city}/{currency} tokens interpolated server-side.';
COMMENT ON COLUMN public.loyalty_messaging.ticker_threshold IS
    'Minimum local dollars reinvested before the impact ticker is shown at all. Below it the ticker hides.';
