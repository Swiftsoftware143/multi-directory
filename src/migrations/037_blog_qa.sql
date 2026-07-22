-- Blog Q&A Automation, Integration Configs, and Newsletter Digests
-- Run: cat 037_blog_qa.sql | PGPASSWORD=SwiftSecure2026! psql -h localhost -U swift -d multidirectory

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
