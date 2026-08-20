-- 078_loyalty_coreswift_lists.sql
-- Per-directory CoreSwift connection + three-list (participant-type) segmentation.
--
-- David's model:
--   - A directory is STANDALONE or part of a NETWORK (ZaarHub = network of directories).
--   - Each directory has 3 participant types, each flowing into its OWN CoreSwift list:
--       users/customers, businesses, suppliers
--   - CoreSwift is the backend comms tool; every signup joins a distinct list + gets tags.
--
-- Adds to `directories` (with network-level fallback):
--   - coreswift_personal_key_encrypted (bytea) + coreswift_base_url — the per-directory
--     personal API key (csk_...) used to POST /api/external/contacts.
--   - coreswift_list_id_{users,businesses,suppliers} — typed list segmentation.

ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_personal_key_encrypted BYTEA;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_base_url VARCHAR(512);

ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_list_id_users UUID;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_list_id_businesses UUID;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_list_id_suppliers UUID;

-- Network-level fallback (shared tenant across a network like ZaarHub)
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_personal_key_encrypted BYTEA;
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_base_url VARCHAR(512);

ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_list_id_users UUID;
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_list_id_businesses UUID;
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_list_id_suppliers UUID;
