-- Add networks table for grouping directories that share branding/theme/root domain
CREATE TABLE IF NOT EXISTS networks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) UNIQUE NOT NULL,
    description TEXT,
    root_domain VARCHAR(255),
    status VARCHAR(20) DEFAULT 'active',
    owner_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Add network_id and url_config to directories
ALTER TABLE directories ADD COLUMN IF NOT EXISTS network_id UUID REFERENCES networks(id) ON DELETE SET NULL;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS url_type VARCHAR(20) DEFAULT 'standalone';
ALTER TABLE directories ADD COLUMN IF NOT EXISTS url_value VARCHAR(255);
ALTER TABLE directories ADD COLUMN IF NOT EXISTS custom_domain VARCHAR(255);

-- Network branding (shared across all directories in the network)
CREATE TABLE IF NOT EXISTS network_branding (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id UUID UNIQUE NOT NULL REFERENCES networks(id) ON DELETE CASCADE,
    logo_url TEXT,
    logo_footer_url TEXT,
    favicon_url TEXT,
    primary_color VARCHAR(7) DEFAULT '#2563eb',
    secondary_color VARCHAR(7) DEFAULT '#64748b',
    accent_color VARCHAR(7) DEFAULT '#f59e0b',
    background_color VARCHAR(7) DEFAULT '#ffffff',
    text_color VARCHAR(7) DEFAULT '#1e293b',
    heading_color VARCHAR(7) DEFAULT '#0f172a',
    heading_font VARCHAR(100) DEFAULT 'Inter',
    body_font VARCHAR(100) DEFAULT 'Inter',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Homepage sections (can belong to a network OR a directory, not both)
CREATE TABLE IF NOT EXISTS homepage_sections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id UUID REFERENCES networks(id) ON DELETE CASCADE,
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    section_type VARCHAR(50) NOT NULL,
    sort_order INT DEFAULT 0,
    title VARCHAR(255),
    subtitle TEXT,
    content TEXT,
    cta_text VARCHAR(100),
    cta_url VARCHAR(500),
    image_url TEXT,
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT homepage_owner_check CHECK (
        (network_id IS NOT NULL AND directory_id IS NULL) OR
        (network_id IS NULL AND directory_id IS NOT NULL)
    )
);

-- Index for homepage lookups
CREATE INDEX IF NOT EXISTS idx_homepage_sections_network ON homepage_sections(network_id) WHERE network_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_homepage_sections_directory ON homepage_sections(directory_id) WHERE directory_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_directories_network ON directories(network_id) WHERE network_id IS NOT NULL;
