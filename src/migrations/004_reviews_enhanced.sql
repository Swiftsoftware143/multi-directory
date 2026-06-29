-- Enhanced reviews table: add moderation, source tracking, directory linkage
ALTER TABLE IF EXISTS reviews ADD COLUMN IF NOT EXISTS reviewer_email TEXT;
ALTER TABLE IF EXISTS reviews ADD COLUMN IF NOT EXISTS status TEXT DEFAULT 'pending';
ALTER TABLE IF EXISTS reviews ADD COLUMN IF NOT EXISTS featured BOOLEAN DEFAULT false;
ALTER TABLE IF EXISTS reviews ADD COLUMN IF NOT EXISTS source TEXT DEFAULT 'direct';
ALTER TABLE IF EXISTS reviews ADD COLUMN IF NOT EXISTS source_url TEXT;
ALTER TABLE IF EXISTS reviews ADD COLUMN IF NOT EXISTS directory_id UUID REFERENCES directories(id) ON DELETE CASCADE;

-- Index for directory-based queries
CREATE INDEX IF NOT EXISTS idx_reviews_status ON reviews(status);
CREATE INDEX IF NOT EXISTS idx_reviews_directory ON reviews(directory_id);
CREATE INDEX IF NOT EXISTS idx_reviews_featured ON reviews(featured) WHERE featured = true;

INSERT INTO _migrations (filename) VALUES ('004_reviews_enhanced.sql')
ON CONFLICT (filename) DO NOTHING;
