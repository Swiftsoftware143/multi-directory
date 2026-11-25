-- 211_plan_checkout_embed.sql
--
-- Plan checkout embed (work-queue "MONETIZATION CHECKOUT EMBED", David 2026-11-24): a plan tier can
-- carry an EMBEDDABLE checkout — an iframe URL or embed snippet — so a business owner buying a paid
-- plan/feature pays INLINE on the directory (smooth reveal) instead of a checkout link that opens a
-- new tab. Mirrors businesses.booking_checkout_url (migration 210).
--
-- This is SEPARATE from plan_sales_page_url: that field is a hosted payment-page link (Stripe/PayPal
-- checkout pages send X-Frame-Options and cannot be iframed), while checkout_embed is an
-- iframe-friendly URL or embed code the operator pastes. When checkout_embed is set, the purchase
-- screen renders it inline; otherwise it falls back to the existing plan_sales_page_url link-out.
--
-- NULL = no embed. An empty string in a write CLEARS it.
--
-- Idempotent: guarded by IF NOT EXISTS so a re-run / re-boot applies nothing twice.

ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS checkout_embed text;

COMMENT ON COLUMN plan_tiers.checkout_embed IS
    'Embeddable checkout for buying this plan: an iframe URL or embed snippet rendered inline on the purchase screen (smooth reveal) instead of a link-out. NULL = none; empty string in a write clears it.';
