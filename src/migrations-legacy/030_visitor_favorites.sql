-- Visitor Favorites / Saved Places
-- Allows logged-in visitors to bookmark their favorite businesses

CREATE TABLE IF NOT EXISTS visitor_favorites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    visitor_account_id UUID NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(visitor_account_id, business_id)
);

CREATE INDEX IF NOT EXISTS idx_visitor_favorites_visitor ON visitor_favorites(visitor_account_id);
CREATE INDEX IF NOT EXISTS idx_visitor_favorites_directory ON visitor_favorites(directory_id);

INSERT INTO _migrations (filename) VALUES ('030_visitor_favorites.sql');
