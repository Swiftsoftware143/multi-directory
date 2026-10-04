-- 144_seo_sitemap_submission.sql
-- ---------------------------------------------------------------------------
-- B91 / B117 gap #10 — nightly sitemap regeneration + search-engine submission.
--
-- The SEO engine already *serves* sitemaps (/sitemap.xml, per-directory and
-- subfolder variants) but nothing ever TOLD a search engine the sitemap changed,
-- and there was no scheduled regeneration at all (the inventory grep for
-- "ping|indexnow|bing" in src/ returned nothing). This adds the two platform
-- objects an admin-configurable submission loop needs:
--
--   * seo_submission_settings — ONE editable row per scope (directory_id NULL =
--     the platform-wide row). is_enabled + cadence_hours + an optional
--     sitemap_url override + ping_targets (a JSON array of {name,url,enabled}
--     where {sitemap} in url is substituted with the encoded sitemap URL).
--     Nothing is hardwired in code: an admin edits the targets in the panel.
--   * seo_submission_log — one row per target hit per run (status code, ok,
--     detail), so the panel can show what was actually submitted and what the
--     engine answered.
--
-- Idempotent (IF NOT EXISTS) so re-applying is a no-op. The default targets are
-- seeded by the handler on first access (data, not code — editable in the panel).
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS public.seo_submission_settings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    is_enabled boolean DEFAULT false NOT NULL,
    cadence_hours integer DEFAULT 24 NOT NULL,
    sitemap_url text,
    ping_targets jsonb DEFAULT '[]'::jsonb NOT NULL,
    last_run_at timestamp with time zone,
    last_status text,
    next_run_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT seo_submission_settings_cadence_check CHECK ((cadence_hours >= 1))
);

CREATE TABLE IF NOT EXISTS public.seo_submission_log (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    target_name text NOT NULL,
    endpoint_url text NOT NULL,
    sitemap_url text NOT NULL,
    status_code integer,
    ok boolean DEFAULT false NOT NULL,
    detail text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

-- Exactly one platform-wide row (directory_id IS NULL) and at most one row per
-- directory, enforced by partial unique indexes so a retry can never duplicate
-- the settings the scheduler reads.
CREATE UNIQUE INDEX IF NOT EXISTS seo_submission_settings_global_uidx
    ON public.seo_submission_settings (directory_id) WHERE directory_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS seo_submission_settings_dir_uidx
    ON public.seo_submission_settings (directory_id) WHERE directory_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS seo_submission_log_created_idx
    ON public.seo_submission_log (created_at DESC);
