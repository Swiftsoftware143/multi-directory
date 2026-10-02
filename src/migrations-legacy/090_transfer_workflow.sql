-- Migration 090: ownership-transfer workflow (T2, round 11 follow-up).
--
-- 086_business_transfers.sql created the transfer + fee tables. This adds what the
-- workflow needs on top of it, WITHOUT touching 086 (it is applied):
--
--   * to_email / from_email  — a transfer can be addressed to an email that has no
--     account yet (an invitation). At accept time the incoming user is bound by
--     matching a real account to that address; nothing is faked in the meantime.
--   * target_directory_id    — where the listing is re-homed when host_stays = false.
--     When host_stays = true the hosting directory (businesses.directory_id) does
--     not change at all. Both are admin-entered on the transfer, never constants.
--   * business_transfer_events — the full audit trail: every transition (created,
--     updated, accepted, declined, cancelled) with actor, role, status change and a
--     metadata blob. business_transfers itself keeps the current state only.
--
-- Every FK is indexed (matches 085's convention).

ALTER TABLE business_transfers ADD COLUMN IF NOT EXISTS to_email text;
ALTER TABLE business_transfers ADD COLUMN IF NOT EXISTS from_email text;
ALTER TABLE business_transfers ADD COLUMN IF NOT EXISTS target_directory_id uuid
    REFERENCES directories(id) ON DELETE SET NULL;
ALTER TABLE business_transfers ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

CREATE INDEX IF NOT EXISTS idx_business_transfers_to_email_lower ON business_transfers(lower(to_email));
CREATE INDEX IF NOT EXISTS idx_business_transfers_from_email_lower ON business_transfers(lower(from_email));
CREATE INDEX IF NOT EXISTS idx_business_transfers_target_directory_id ON business_transfers(target_directory_id);
CREATE INDEX IF NOT EXISTS idx_business_transfers_created_at ON business_transfers(created_at DESC);

CREATE TABLE IF NOT EXISTS business_transfer_events (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    transfer_id   uuid NOT NULL REFERENCES business_transfers(id) ON DELETE CASCADE,
    business_id   uuid REFERENCES businesses(id) ON DELETE SET NULL,
    actor_user_id uuid,
    actor_role    varchar(32),
    event         varchar(32) NOT NULL,
    from_status   varchar(20),
    to_status     varchar(20),
    metadata      jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_business_transfer_events_transfer_id ON business_transfer_events(transfer_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_events_business_id ON business_transfer_events(business_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_events_actor_user_id ON business_transfer_events(actor_user_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_events_created_at ON business_transfer_events(created_at DESC);
