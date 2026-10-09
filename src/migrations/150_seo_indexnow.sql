-- 150_seo_indexnow.sql
-- ---------------------------------------------------------------------------
-- B142 — the legacy search-engine sitemap ping endpoints are DEAD. Measured
-- 2026-10-08 from this box:
--     GET https://www.google.com/ping?sitemap=... -> 404
--         ("Sitemaps ping is deprecated. See .../sitemaps-lastmod-ping")
--     GET https://www.bing.com/ping?sitemap=...   -> 410 Gone
-- So the nightly submission loop built in migration 144 could never actually
-- succeed against its seeded targets. The live replacement for both engines is
-- IndexNow (Bing, Yandex, Seznam, Naver): a POST to https://api.indexnow.org/indexnow
-- with {host, key, keyLocation, urlList}. The engine re-fetches the key from
-- keyLocation on the SAME host before it trusts the urls.
--
-- This adds the one platform-level knob that requires: an admin-editable key.
-- An empty/NULL key disables the IndexNow leg (the generic GET targets in
-- ping_targets remain, so nothing is hardwired). The key file is served by the
-- app itself at /api/v1/seo/indexnow-key.txt (public, no credential).
--
-- Idempotent so re-applying is a no-op.
-- ---------------------------------------------------------------------------

ALTER TABLE public.seo_submission_settings
    ADD COLUMN IF NOT EXISTS indexnow_key text;
