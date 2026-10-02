-- BL13: Memberships & Subscriber Dashboard
-- Phase 1 Task 1 for ZaarHub community commerce

-- 1. Add user_id to claimed_businesses (links to users table)
ALTER TABLE claimed_businesses
ADD COLUMN IF NOT EXISTS user_id UUID REFERENCES users(id) ON DELETE SET NULL;

-- 2. Add feature_config to directories
ALTER TABLE directories
ADD COLUMN IF NOT EXISTS feature_config JSONB DEFAULT '{"deals":true,"blogging":true,"community_posts":true,"b2b_marketplace":false,"visitor_accounts":true,"gamification":false}'::jsonb;

-- 3. Create visitor_accounts table (community visitor accounts, distinct from tracking visitors)
CREATE TABLE IF NOT EXISTS visitor_accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    name TEXT,
    phone TEXT,
    directory_id UUID REFERENCES directories(id) ON DELETE SET NULL,
    is_active BOOLEAN DEFAULT true,
    last_login_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for visitor_accounts lookups
CREATE INDEX IF NOT EXISTS idx_visitor_accounts_email ON visitor_accounts(email);
CREATE INDEX IF NOT EXISTS idx_visitor_accounts_directory ON visitor_accounts(directory_id);
