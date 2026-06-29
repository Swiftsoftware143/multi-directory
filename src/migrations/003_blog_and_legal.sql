-- Blog posts table
CREATE TABLE IF NOT EXISTS blog_posts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    slug TEXT,
    excerpt TEXT,
    content TEXT NOT NULL,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    published BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_blog_posts_directory ON blog_posts(directory_id);
CREATE INDEX IF NOT EXISTS idx_blog_posts_published ON blog_posts(published);

-- Legal pages table
CREATE TABLE IF NOT EXISTS legal_pages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    page_type TEXT NOT NULL DEFAULT 'custom',
    content TEXT NOT NULL,
    published BOOLEAN DEFAULT true,
    is_global BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_legal_pages_type ON legal_pages(page_type);
CREATE INDEX IF NOT EXISTS idx_legal_pages_published ON legal_pages(published);
