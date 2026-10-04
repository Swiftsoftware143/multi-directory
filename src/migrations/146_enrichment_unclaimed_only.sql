-- B83 (auto-refresh scope): the rotating enrichment cycle must only touch UNCLAIMED
-- listings by default. Once a real owner manages a listing their data is authoritative and
-- automation must not overwrite it. `unclaimed_only` defaults TRUE so an existing install gets
-- the safe behaviour on the next boot; an admin can turn it off in the Data Enrichment card.
ALTER TABLE enrichment_settings
    ADD COLUMN IF NOT EXISTS unclaimed_only boolean NOT NULL DEFAULT true;
