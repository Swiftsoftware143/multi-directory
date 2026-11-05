-- 175_seo_submission_urls.sql — submit only URLs the search engine has not been given yet
-- (item 12 reopened: repeated "Run now" / scheduled runs resubmitted all ~4 030 sitemap URLs,
-- and IndexNow answered HTTP 429 "TooManyRequests" — measured 2026-11-05 19:30, the operator's
-- card then showed the run as failed).
--
-- IndexNow's own guidance is to submit NEW or CHANGED urls, never the same set again. The run
-- therefore has to remember what it already submitted. One row per url per scope:
--   * scope = 'platform' for the platform-wide settings row (directory_id IS NULL),
--     or the directory's uuid as text for a per-directory row.
-- The delta (sitemap urls minus these rows) is what goes on the wire, so an unchanged sitemap
-- costs zero requests and one new listing costs a one-url request.
CREATE TABLE IF NOT EXISTS seo_submission_urls (
    id                  bigserial PRIMARY KEY,
    scope               text        NOT NULL,
    url                 text        NOT NULL,
    first_submitted_at  timestamptz NOT NULL DEFAULT now()
);

-- Idempotent for a re-run; the handler inserts with ON CONFLICT DO NOTHING.
CREATE UNIQUE INDEX IF NOT EXISTS seo_submission_urls_scope_url_uq
    ON seo_submission_urls (scope, url);
