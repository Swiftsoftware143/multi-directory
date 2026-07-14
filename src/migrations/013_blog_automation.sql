-- Blog automation upgrade: templates, scheduling, multi-city distribution
-- Part of multi-directory Phase 5

-- Blog post templates
CREATE TABLE IF NOT EXISTS blog_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    description TEXT,
    category TEXT NOT NULL DEFAULT 'seo',  -- 'seo', 'geo', 'aeo', 'listicle', 'howto', 'faq', 'guide', 'news'
    content_template TEXT NOT NULL,
    merge_fields JSONB DEFAULT '[]'::jsonb,  -- configurable fields like [{"key":"city","label":"City"}, ...]
    is_global BOOLEAN DEFAULT true,
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,  -- NULL = global, set = per-directory
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_blog_templates_category ON blog_templates(category);
CREATE INDEX IF NOT EXISTS idx_blog_templates_directory ON blog_templates(directory_id);

-- Add scheduling + distribution to blog_posts
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS scheduled_at TIMESTAMPTZ;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS template_id UUID REFERENCES blog_templates(id) ON DELETE SET NULL;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS template_data JSONB DEFAULT '{}'::jsonb;  -- merge field values
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS is_master BOOLEAN DEFAULT false;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS master_post_id UUID REFERENCES blog_posts(id) ON DELETE SET NULL;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS blog_category TEXT DEFAULT 'general';  -- seo, geo, listicle, howto, faq, guide, news
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS tags TEXT[] DEFAULT '{}';
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS meta_description TEXT;
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS feature_image TEXT;

-- Newsletter queue
CREATE TABLE IF NOT EXISTS newsletter_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    intro_text TEXT,
    include_blog BOOLEAN DEFAULT true,
    include_deals BOOLEAN DEFAULT true,
    manual_sections JSONB DEFAULT '[]'::jsonb,  -- manual ad spots / custom content
    scheduled_at TIMESTAMPTZ,
    sent_at TIMESTAMPTZ,
    status TEXT DEFAULT 'draft',  -- 'draft', 'scheduled', 'sent'
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
