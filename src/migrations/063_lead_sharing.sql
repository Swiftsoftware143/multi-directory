-- Migration 063: Lead Sharing Network — shared leads pool
-- Businesses share leads they can't fulfill

CREATE TABLE IF NOT EXISTS shared_leads (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT,
    location TEXT,
    estimated_value NUMERIC(10,2),
    source TEXT,                    -- 'referral', 'overflow', 'not_serviced'
    poster_business_id UUID NOT NULL REFERENCES businesses(id),
    status TEXT NOT NULL DEFAULT 'available',  -- available, claimed, fulfilled, expired
    claimed_by UUID REFERENCES businesses(id),
    claimed_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_shared_leads_status ON shared_leads(status);
CREATE INDEX idx_shared_leads_poster ON shared_leads(poster_business_id);
CREATE INDEX idx_shared_leads_category ON shared_leads(category);

-- Lead sharing credit/tracking
CREATE TABLE IF NOT EXISTS lead_share_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    lead_id UUID NOT NULL REFERENCES shared_leads(id),
    from_business_id UUID NOT NULL REFERENCES businesses(id),
    to_business_id UUID NOT NULL REFERENCES businesses(id),
    status TEXT NOT NULL DEFAULT 'transferred',  -- transferred, accepted, completed, disputed
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_lead_share_transactions_lead ON lead_share_transactions(lead_id);
CREATE INDEX idx_lead_share_transactions_from ON lead_share_transactions(from_business_id);
CREATE INDEX idx_lead_share_transactions_to ON lead_share_transactions(to_business_id);
