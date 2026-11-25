-- 214_business_placements.sql
--
-- Satellite cities (work-queue t_5306e167): a PAID business appears in its home city PLUS its
-- satellite cities — the other directories in the same `region` (Featured, capped by max_listings)
-- or every directory in the network (Premium). The appearance is a SEPARATE ROW here
-- (business_id + directory_id), never a duplicate `businesses` row, so reviews/photos/rating stay
-- on the single business record and COUNT(businesses) never changes when a business is cross-listed.
--
-- The HOME city is implicit (businesses.directory_id) and is NOT written here; only SATELLITE
-- (non-home) placements are. Lapsing a subscription removes these rows and the home listing remains.
--
-- Idempotent: guarded by IF NOT EXISTS so a re-run / re-boot applies nothing twice.

CREATE TABLE IF NOT EXISTS business_placements (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id  uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id uuid NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    created_at   timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT business_placements_business_directory_uniq UNIQUE (business_id, directory_id)
);

CREATE INDEX IF NOT EXISTS business_placements_directory_idx ON business_placements (directory_id);

COMMENT ON TABLE business_placements IS
    'Satellite-city placements: a paid business''s non-home directory appearances. Rows are written when a subscription activates and removed on lapse. Home appearances are implicit (businesses.directory_id).';
