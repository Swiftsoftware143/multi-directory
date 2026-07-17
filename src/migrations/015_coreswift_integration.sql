-- CoreSwift integration columns for networks and directories
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_tenant_id UUID;
ALTER TABLE networks ADD COLUMN IF NOT EXISTS coreswift_key_prefix TEXT;

ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_tenant_id UUID;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_key_prefix TEXT;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_list_id_claimed UUID;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS coreswift_list_id_newsletter UUID;
