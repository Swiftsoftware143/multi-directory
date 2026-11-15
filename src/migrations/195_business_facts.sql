-- Business facts: the first-hand details a draft must actually use.
--
-- Without these a generated post reads like every other post on the internet. Each row is one
-- fact about the business (a service, an area it serves, a real price, a job it did, an owner
-- quote, a credential). Drafting is told to use them; the quality score checks that it did.

CREATE TABLE IF NOT EXISTS business_facts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID REFERENCES businesses(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'service', 'area', 'price', 'job', 'quote', 'photo', 'credential', 'differentiator'
    )),
    content TEXT NOT NULL,
    source TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_business_facts_directory
    ON business_facts (directory_id, is_active);
CREATE INDEX IF NOT EXISTS idx_business_facts_business
    ON business_facts (business_id);
