-- Migration 060: RFQ Marketplace — Request for Quote system
-- Businesses post what they need, suppliers bid

-- Core RFQ table
CREATE TABLE IF NOT EXISTS rfqs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT,           -- e.g., 'produce', 'equipment', 'services', 'packaging'
    quantity TEXT,            -- flexible: "500 units", "ongoing", "per project"
    budget_min NUMERIC(10,2),
    budget_max NUMERIC(10,2),
    deadline DATE,
    delivery_location TEXT,
    poster_business_id UUID NOT NULL REFERENCES businesses(id),
    status TEXT NOT NULL DEFAULT 'open',  -- open, in_review, awarded, closed, cancelled
    urgency TEXT DEFAULT 'standard',      -- low, standard, high, urgent
    is_public BOOLEAN NOT NULL DEFAULT true,
    awarded_to UUID REFERENCES businesses(id),
    awarded_bid_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ,
    view_count INTEGER DEFAULT 0
);
CREATE INDEX idx_rfqs_status ON rfqs(status);
CREATE INDEX idx_rfqs_poster ON rfqs(poster_business_id);
CREATE INDEX idx_rfqs_category ON rfqs(category);
CREATE INDEX idx_rfqs_deadline ON rfqs(deadline) WHERE status = 'open';

-- Bids on RFQs
CREATE TABLE IF NOT EXISTS rfq_bids (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rfq_id UUID NOT NULL REFERENCES rfqs(id) ON DELETE CASCADE,
    bidder_business_id UUID NOT NULL REFERENCES businesses(id),
    amount NUMERIC(10,2) NOT NULL,
    details TEXT NOT NULL,
    delivery_timeline TEXT,
    status TEXT NOT NULL DEFAULT 'submitted',  -- submitted, withdrawn, accepted, rejected
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_rfq_bids_rfq ON rfq_bids(rfq_id);
CREATE INDEX idx_rfq_bids_bidder ON rfq_bids(bidder_business_id);

-- RFQ messages / Q&A
CREATE TABLE IF NOT EXISTS rfq_messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    rfq_id UUID NOT NULL REFERENCES rfqs(id) ON DELETE CASCADE,
    sender_business_id UUID NOT NULL REFERENCES businesses(id),
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_rfq_messages_rfq ON rfq_messages(rfq_id);
