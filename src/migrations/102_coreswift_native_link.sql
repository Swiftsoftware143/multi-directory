-- 102 CoreSwift native link — carry the identity across.
--
-- David's model: a NETWORK or a STANDALONE DIRECTORY connects its own CoreSwift account
-- (tenant + base URL + optional personal key + typed list ids, all on the row itself and
-- written from the Admin Panel). The link columns already existed; this migration adds the
-- return path — the hub's contact id on the Multi-Directory side — so the SAME person is ONE
-- contact in CoreSwift, linked to their MD visitor account, their business and the onboarding
-- response the admin drills into (card B68).
--
-- Additive and idempotent: safe to re-run, no data touched, no column dropped.

ALTER TABLE visitor_accounts ADD COLUMN IF NOT EXISTS coreswift_contact_id uuid;
ALTER TABLE businesses ADD COLUMN IF NOT EXISTS coreswift_contact_id uuid;
ALTER TABLE survey_responses ADD COLUMN IF NOT EXISTS coreswift_contact_id uuid;

CREATE INDEX IF NOT EXISTS idx_visitor_accounts_coreswift_contact
    ON visitor_accounts (coreswift_contact_id)
    WHERE coreswift_contact_id IS NOT NULL;
