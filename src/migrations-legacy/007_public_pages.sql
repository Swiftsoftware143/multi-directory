-- Public Pages & Themes for Multi-Directory

CREATE TABLE IF NOT EXISTS landing_pages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    directory_id UUID REFERENCES directories(id),
    hero_title TEXT,
    hero_subtitle TEXT,
    hero_cta_text TEXT,
    hero_cta_url TEXT,
    features JSONB DEFAULT '[]',
    testimonials JSONB DEFAULT '[]',
    faq JSONB DEFAULT '[]',
    seo_title TEXT,
    seo_description TEXT,
    published BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS public_themes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    directory_id UUID REFERENCES directories(id),
    primary_color TEXT DEFAULT '#2563eb',
    secondary_color TEXT DEFAULT '#1e40af',
    header_style TEXT DEFAULT 'gradient',
    layout TEXT DEFAULT 'grid',
    show_search BOOLEAN DEFAULT true,
    show_categories BOOLEAN DEFAULT true,
    show_featured BOOLEAN DEFAULT true,
    items_per_page INTEGER DEFAULT 12,
    custom_css TEXT,
    custom_js TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
