-- Public web address history for a listing (card W19).
--
-- A listing's public page is /<city>/businesses/<slug>. The owner can change that
-- address from the portal, but every link, search result and printed QR code that
-- points at the OLD address must keep working — otherwise renaming a listing throws
-- away whatever ranking and traffic it had earned. Each superseded address is kept
-- here and the page answers a permanent (301) redirect to the current one.
--
-- (directory_id, slug) is unique: one address can only ever resolve to one listing,
-- so an old address can never be "reused" by a different business and inherit its
-- reputation.

CREATE TABLE IF NOT EXISTS business_slug_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL,
    slug VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT business_slug_history_directory_slug_key UNIQUE (directory_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_business_slug_history_business
    ON business_slug_history (business_id);
