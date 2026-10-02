-- Business tagging for content tracking
-- Track which businesses are mentioned in blog posts and programmatic pages
-- This powers the business dashboard "mentions" metrics

ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS mentioned_business_ids UUID[] NOT NULL DEFAULT '{}';
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS mentioned_business_ids UUID[] NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_blog_posts_mentioned_biz ON blog_posts USING GIN(mentioned_business_ids);
CREATE INDEX IF NOT EXISTS idx_programmatic_pages_mentioned_biz ON programmatic_pages USING GIN(mentioned_business_ids);

-- Business articles already have business_id, just index for fast aggregation
CREATE INDEX IF NOT EXISTS idx_business_articles_biz_id ON business_articles(business_id);
