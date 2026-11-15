-- Rankifyr-parity guardrails for the in-house blog engine.
--
-- Publishing becomes a gate instead of a button: every draft is checked, scored and given
-- pass | hold | block. Review is the default; auto-publish only ever publishes a pass inside
-- a per-directory daily cap, and only when the directory turns it on.

ALTER TABLE directories
    ADD COLUMN IF NOT EXISTS blog_publish_mode TEXT NOT NULL DEFAULT 'review',
    ADD COLUMN IF NOT EXISTS blog_auto_publish_daily_cap INT NOT NULL DEFAULT 3,
    ADD COLUMN IF NOT EXISTS blog_min_quality_score INT NOT NULL DEFAULT 70,
    ADD COLUMN IF NOT EXISTS blog_ai_disclosure BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS blog_guardrail_settings JSONB NOT NULL DEFAULT '{}'::jsonb;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'directories_blog_publish_mode_check'
    ) THEN
        ALTER TABLE directories
            ADD CONSTRAINT directories_blog_publish_mode_check
            CHECK (blog_publish_mode IN ('review', 'auto'));
    END IF;
END $$;

ALTER TABLE blog_posts
    ADD COLUMN IF NOT EXISTS guardrail_outcome TEXT,
    ADD COLUMN IF NOT EXISTS quality_score INT,
    ADD COLUMN IF NOT EXISTS guardrail_checked_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS guardrail_content_hash TEXT,
    ADD COLUMN IF NOT EXISTS reviewed_by TEXT,
    ADD COLUMN IF NOT EXISTS reviewed_at TIMESTAMPTZ;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'blog_posts_guardrail_outcome_check'
    ) THEN
        ALTER TABLE blog_posts
            ADD CONSTRAINT blog_posts_guardrail_outcome_check
            CHECK (guardrail_outcome IS NULL OR guardrail_outcome IN ('pass', 'hold', 'block'));
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS guardrail_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    post_id UUID NOT NULL REFERENCES blog_posts(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('pass', 'hold', 'block')),
    quality_score INT NOT NULL DEFAULT 0,
    checks JSONB NOT NULL DEFAULT '[]'::jsonb,
    content_hash TEXT NOT NULL,
    ran_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_guardrail_runs_post ON guardrail_runs (post_id, ran_at DESC);
