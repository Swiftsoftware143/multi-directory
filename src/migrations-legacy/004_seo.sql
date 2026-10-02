-- SEO meta table for per-page metadata
CREATE TABLE IF NOT EXISTS seo_meta (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    page_type TEXT NOT NULL,
    page_id UUID,
    title TEXT,
    description TEXT,
    keywords TEXT,
    og_image TEXT,
    og_title TEXT,
    og_description TEXT,
    schema_type TEXT,
    custom_schema JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(page_type, page_id)
);

CREATE INDEX IF NOT EXISTS idx_seo_meta_page ON seo_meta(page_type, page_id);

-- Sitemap configuration per directory
CREATE TABLE IF NOT EXISTS sitemap_config (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    auto_generate BOOLEAN DEFAULT true,
    priority DECIMAL(2,1) DEFAULT 0.5,
    change_freq TEXT DEFAULT 'weekly',
    last_generated TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(directory_id)
);

CREATE INDEX IF NOT EXISTS idx_sitemap_config_dir ON sitemap_config(directory_id);
