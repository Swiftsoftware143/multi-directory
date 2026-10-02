-- Category request system for Phase 2 multi-category
-- Business owners request new categories; admins approve/deny

CREATE TABLE IF NOT EXISTS category_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES directory_categories(id) ON DELETE CASCADE,
    requested_by UUID, -- user who made the request
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied')),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_category_requests_business ON category_requests(business_id);
CREATE INDEX IF NOT EXISTS idx_category_requests_status ON category_requests(status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_category_requests_unique ON category_requests(business_id, category_id) WHERE status = 'pending';
