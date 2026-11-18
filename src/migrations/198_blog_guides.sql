-- Guides: a pillar post ("the Palm Bay homeowner's guide") gathers supporting posts.
-- Additive only: both columns default to "no guide", so existing posts are untouched.
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS is_pillar BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS pillar_id UUID REFERENCES blog_posts(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_blog_posts_pillar ON blog_posts(pillar_id);
