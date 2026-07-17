-- Migration 019: Business SEO Articles
-- Allows paid and directory-owner articles for SEO targeting specific keywords

CREATE TABLE IF NOT EXISTS business_articles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID REFERENCES businesses(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    slug TEXT NOT NULL,
    keyword TEXT NOT NULL,
    meta_description TEXT,
    content TEXT,
    status TEXT DEFAULT 'draft',
    impressions INTEGER DEFAULT 0,
    clicks INTEGER DEFAULT 0,
    is_owner_article BOOLEAN DEFAULT FALSE,  -- FALSE = paid business article, TRUE = directory owner's free SEO
    subscription_active BOOLEAN DEFAULT FALSE,
    subscription_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_business_articles_directory ON business_articles(directory_id);
CREATE INDEX IF NOT EXISTS idx_business_articles_slug ON business_articles(slug);
CREATE UNIQUE INDEX IF NOT EXISTS idx_business_articles_dir_slug ON business_articles(directory_id, slug);
