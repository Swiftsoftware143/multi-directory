-- Migration 040: Sponsor & Ad Management System
-- Self-serve ad marketplace with approval flow

-- ── Sponsors (one per business per directory) ──
CREATE TABLE IF NOT EXISTS sponsors (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','active','suspended','inactive')),
    commission_rate DECIMAL(5,2) DEFAULT 0,
    notes TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(directory_id, business_id)
);

-- ── Ad Creatives (image assets per sponsor, locked to slot dimensions) ──
CREATE TABLE IF NOT EXISTS ad_creatives (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sponsor_id UUID NOT NULL REFERENCES sponsors(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    image_url TEXT NOT NULL,
    target_url TEXT,
    width INTEGER NOT NULL CHECK (width > 0),
    height INTEGER NOT NULL CHECK (height > 0),
    mime_type TEXT DEFAULT 'image/png',
    file_size_bytes INTEGER,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected','archived')),
    rejection_reason TEXT,
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for looking up creatives by dimensions (for slot matching)
CREATE INDEX IF NOT EXISTS idx_ad_creatives_sponsor ON ad_creatives(sponsor_id);
CREATE INDEX IF NOT EXISTS idx_ad_creatives_dims ON ad_creatives(width, height);
CREATE INDEX IF NOT EXISTS idx_ad_creatives_status ON ad_creatives(status);

-- ── Ad Schedules (connects sponsor + creative + zone slot + dates) ──
CREATE TABLE IF NOT EXISTS ad_schedules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    ad_zone_id UUID NOT NULL REFERENCES ad_zones(id) ON DELETE CASCADE,
    sponsor_id UUID NOT NULL REFERENCES sponsors(id) ON DELETE CASCADE,
    creative_id UUID NOT NULL REFERENCES ad_creatives(id) ON DELETE CASCADE,
    start_date TIMESTAMPTZ NOT NULL,
    end_date TIMESTAMPTZ NOT NULL CHECK (end_date > start_date),
    price_monthly DECIMAL(10,2) NOT NULL DEFAULT 0.00,
    total_price DECIMAL(10,2) NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','active','completed','cancelled')),
    auto_renew BOOLEAN DEFAULT false,
    created_by UUID, -- user who created the schedule
    approved_at TIMESTAMPTZ,
    approved_by UUID,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_dates ON ad_schedules(start_date, end_date);
CREATE INDEX IF NOT EXISTS idx_ad_schedules_zone ON ad_schedules(ad_zone_id);
CREATE INDEX IF NOT EXISTS idx_ad_schedules_sponsor ON ad_schedules(sponsor_id);
CREATE INDEX IF NOT EXISTS idx_ad_schedules_status ON ad_schedules(status);
CREATE INDEX IF NOT EXISTS idx_ad_schedules_directory ON ad_schedules(directory_id);


-- ── Earnings Tracking ──
CREATE TABLE IF NOT EXISTS ad_earnings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES ad_schedules(id) ON DELETE CASCADE,
    sponsor_id UUID NOT NULL REFERENCES sponsors(id) ON DELETE CASCADE,
    ad_zone_id UUID NOT NULL REFERENCES ad_zones(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    amount DECIMAL(10,2) NOT NULL,
    period_start DATE NOT NULL,
    period_end DATE NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','paid','overdue','cancelled')),
    paid_at TIMESTAMPTZ,
    notes TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_sponsor ON ad_earnings(sponsor_id);
CREATE INDEX IF NOT EXISTS idx_ad_earnings_zone ON ad_earnings(ad_zone_id);
CREATE INDEX IF NOT EXISTS idx_ad_earnings_directory ON ad_earnings(directory_id);

-- ── Approval Queue (unified for all monetization) ──
CREATE TABLE IF NOT EXISTS approval_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL CHECK (item_type IN ('sponsor','ad_creative','ad_schedule','featured_listing','subscription')),
    item_id UUID NOT NULL,
    submitted_by UUID,
    submitted_at TIMESTAMPTZ DEFAULT NOW(),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected')),
    reviewed_by UUID,
    reviewed_at TIMESTAMPTZ,
    notes TEXT,
    UNIQUE(item_type, item_id)
);

CREATE INDEX IF NOT EXISTS idx_approval_queue_directory ON approval_queue(directory_id);
CREATE INDEX IF NOT EXISTS idx_approval_queue_status ON approval_queue(status);
