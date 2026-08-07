-- Migration 065: Co-op Buying Groups — group purchasing with shared deals

CREATE TABLE IF NOT EXISTS buying_groups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    description TEXT,
    category TEXT,              -- 'produce', 'equipment', 'supplies', etc.
    founder_business_id UUID NOT NULL REFERENCES businesses(id),
    status TEXT NOT NULL DEFAULT 'recruiting',  -- recruiting, active, closed
    member_count INTEGER DEFAULT 1,
    min_members INTEGER DEFAULT 2,
    max_members INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_buying_groups_status ON buying_groups(status);
CREATE INDEX idx_buying_groups_category ON buying_groups(category);
CREATE INDEX idx_buying_groups_founder ON buying_groups(founder_business_id);

-- Group members
CREATE TABLE IF NOT EXISTS buying_group_members (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id UUID NOT NULL REFERENCES buying_groups(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id),
    role TEXT NOT NULL DEFAULT 'member',  -- founder, admin, member
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(group_id, business_id)
);
CREATE INDEX idx_buying_group_members_group ON buying_group_members(group_id);
CREATE INDEX idx_buying_group_members_business ON buying_group_members(business_id);

-- Group deals (collective purchasing deals)
CREATE TABLE IF NOT EXISTS buying_group_deals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id UUID NOT NULL REFERENCES buying_groups(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT,
    supplier_business_id UUID NOT NULL REFERENCES businesses(id),
    product_name TEXT NOT NULL,
    normal_price NUMERIC(10,2),
    group_price NUMERIC(10,2) NOT NULL,
    min_quantity INTEGER NOT NULL,     -- minimum order required to unlock deal
    current_quantity INTEGER DEFAULT 0,
    unit TEXT DEFAULT 'each',
    deadline DATE,
    status TEXT NOT NULL DEFAULT 'active',  -- active, funded, cancelled
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_buying_group_deals_group ON buying_group_deals(group_id);
CREATE INDEX idx_buying_group_deals_status ON buying_group_deals(status);

-- Individual commitments to group deals
CREATE TABLE IF NOT EXISTS group_deal_commitments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    deal_id UUID NOT NULL REFERENCES buying_group_deals(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id),
    quantity INTEGER NOT NULL,
    total_amount NUMERIC(10,2),
    status TEXT NOT NULL DEFAULT 'committed',  -- committed, paid, delivered
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_group_deal_commitments_deal ON group_deal_commitments(deal_id);
CREATE INDEX idx_group_deal_commitments_business ON group_deal_commitments(business_id);
