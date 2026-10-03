-- 130_coreswift_entity_sync.sql — native per-entity CoreSwift sync state (card B111).
--
-- David's binding principle (2026-10-02): Multi-Directory is a DATA COMPANY that presents as a
-- directory. A user/customer, a local business and a supplier are each a COMPLETE record, and the
-- one external integration — CoreSwift, the hub — must be able to receive that record per entity.
--
-- Only some capture paths pushed before this card (a claimed business, a sponsor, a newsletter
-- signup). There was no per-record answer to "was this entity pushed?" — the per-record markers were
-- scattered (`businesses.coreswift_contact_id`, `visitor_accounts.coreswift_contact_id`,
-- `survey_responses.coreswift_pushed/_push_error`).
--
-- This table is the ONE place that answers it for every entity kind: one row per (kind, entity),
-- carrying the push status, the hub contact id, when it last succeeded and the last real error.
-- It is what makes a push-one / push-all run RESUNABLE (a failed row stays `error`, a pushed row is
-- `synced` and is skipped next time unless forced) and IDEMPOTENT at the MD layer (a `synced` row is
-- not pushed again).
--
-- entity_id is deliberately NOT a foreign key: the table is polymorphic across businesses and
-- visitor_accounts, so it cannot point at one parent. directory_id is a real FK so a directory
-- delete cannot leave orphaned sync state behind.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

CREATE TABLE IF NOT EXISTS coreswift_sync_state (
    id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_kind          text NOT NULL
                         CHECK (entity_kind IN ('business', 'supplier', 'customer')),
    entity_id            uuid NOT NULL,
    directory_id         uuid NULL REFERENCES directories(id) ON DELETE CASCADE,
    status               text NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending', 'synced', 'error', 'not_configured')),
    coreswift_contact_id uuid NULL,
    attempts             integer NOT NULL DEFAULT 0,
    last_pushed_at       timestamptz NULL,
    last_error           text NULL,
    updated_at           timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT coreswift_sync_state_entity_uniq UNIQUE (entity_kind, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_coreswift_sync_state_dir
    ON coreswift_sync_state (directory_id, entity_kind, status);
