-- Blog content decay detection columns (used by /api/v1/blog/decay/* handlers)
-- These were referenced by src/handlers/blog_features.rs but never added to the schema.

ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS last_refreshed TIMESTAMPTZ;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS page_views INTEGER NOT NULL DEFAULT 0;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS traffic_trend TEXT;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS decay_flag BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS refresh_priority TEXT;

CREATE INDEX IF NOT EXISTS idx_blog_posts_decay_flag ON blog_posts(decay_flag);
CREATE INDEX IF NOT EXISTS idx_blog_posts_refresh_priority ON blog_posts(refresh_priority);

INSERT INTO _migrations (filename) VALUES ('075_blog_content_decay.sql');
