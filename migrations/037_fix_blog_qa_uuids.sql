-- Fix migration 036: use UUID for directory_id and blog_post_id instead of INTEGER
-- Since directories.id and blog_posts.id are UUID, not INTEGER

-- Drop the incorrectly-typed tables from 036
DROP TABLE IF EXISTS blog_qa_posts CASCADE;
DROP TABLE IF EXISTS blog_qa_keywords CASCADE;
DROP TABLE IF EXISTS newsletter_digests CASCADE;

-- Recreate with correct UUID types

CREATE TABLE IF NOT EXISTS blog_qa_keywords (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    keyword TEXT NOT NULL,
    intent TEXT DEFAULT 'question',
    source TEXT DEFAULT 'manual',
    frequency INTEGER DEFAULT 0,
    target_category TEXT,
    status TEXT DEFAULT 'unused',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS blog_qa_posts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    blog_post_id UUID REFERENCES blog_posts(id) ON DELETE SET NULL,
    question TEXT NOT NULL,
    keyword TEXT NOT NULL,
    ai_model TEXT,
    template_id TEXT,
    status TEXT DEFAULT 'draft',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS newsletter_digests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT,
    source_post_ids UUID[] DEFAULT '{}',
    status TEXT DEFAULT 'draft',
    scheduled_at TIMESTAMPTZ,
    sent_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS integration_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider TEXT NOT NULL UNIQUE,
    config JSONB NOT NULL DEFAULT '{}',
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_blog_qa_keywords_dir ON blog_qa_keywords(directory_id);
CREATE INDEX IF NOT EXISTS idx_blog_qa_keywords_status ON blog_qa_keywords(status);
CREATE INDEX IF NOT EXISTS idx_blog_qa_posts_dir ON blog_qa_posts(directory_id);
CREATE INDEX IF NOT EXISTS idx_newsletter_digests_dir ON newsletter_digests(directory_id);
