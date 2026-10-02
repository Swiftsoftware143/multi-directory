-- 016_coreswift_sponsors_list.sql
-- Add sponsors list column to CoreSwift integration

ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_list_id_sponsors UUID;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_list_id_sponsors UUID;

-- Also ensure claimed/newsletter columns exist on networks (pre-existing on directories)
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_list_id_claimed UUID;
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_list_id_newsletter UUID;
