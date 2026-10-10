-- 166_company_headquarters.sql — the company's Headquarters (David's MD STRUCTURE rule, 2026-10-10).
--
-- ONE value for the whole company ("Orlando, Florida"; city+state is enough), shown on EVERY city
-- page. A city needs no address of its own, and this is entered once. It lives on the company/brand
-- record (zaarhub_site_config) alongside site_name and the contact details, so every city inherits
-- the same value at render time (merge_fields::MergeContext::for_network, and the public city page
-- GET /api/v1/zaarhub/cities/:slug which exposes it as `directory.headquarters`).
--
-- Nullable and additive: an existing deploy keeps working; a blank is simply not rendered.
ALTER TABLE zaarhub_site_config ADD COLUMN IF NOT EXISTS headquarters varchar(255);
