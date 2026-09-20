-- Migration 086: business ownership transfers (T2).
--
-- Ownership transfers did not exist in any form: no table, no route, no UI.
-- A transfer is initiated by the current owner (or an admin), then accepted or
-- declined by the incoming owner. Nothing about it is hardwired:
--   * fee_cents, currency and host_stays are admin/owner-entered values;
--   * fee_direction says who pays whom (the spec's "fees to the new owner" is a
--     choice an admin makes per transfer, not a constant baked into code).
-- Every FK is indexed (matches migration 085's convention).

CREATE TABLE IF NOT EXISTS business_transfers (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id     uuid NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    from_tenant_id  uuid,
    from_user_id    uuid,
    to_tenant_id    uuid,
    to_user_id      uuid,
    fee_cents       integer NOT NULL DEFAULT 0,
    currency        varchar(8) NOT NULL DEFAULT 'USD',
    -- who pays the fee: 'incoming' (new owner pays outgoing), 'outgoing' (previous
    -- owner pays), 'platform' (platform absorbs it — no ledger row is created).
    fee_direction   varchar(16) NOT NULL DEFAULT 'incoming',
    host_stays      boolean NOT NULL DEFAULT true,
    status          varchar(20) NOT NULL DEFAULT 'pending',
    notes           text,
    requested_by    uuid,
    created_at      timestamptz NOT NULL DEFAULT now(),
    decided_at      timestamptz,
    CONSTRAINT business_transfers_status_check
        CHECK (status IN ('pending', 'accepted', 'declined', 'cancelled')),
    CONSTRAINT business_transfers_fee_direction_check
        CHECK (fee_direction IN ('incoming', 'outgoing', 'platform')),
    CONSTRAINT business_transfers_fee_check CHECK (fee_cents >= 0)
);

CREATE INDEX IF NOT EXISTS idx_business_transfers_business_id ON business_transfers(business_id);
CREATE INDEX IF NOT EXISTS idx_business_transfers_from_tenant_id ON business_transfers(from_tenant_id);
CREATE INDEX IF NOT EXISTS idx_business_transfers_from_user_id ON business_transfers(from_user_id);
CREATE INDEX IF NOT EXISTS idx_business_transfers_to_tenant_id ON business_transfers(to_tenant_id);
CREATE INDEX IF NOT EXISTS idx_business_transfers_to_user_id ON business_transfers(to_user_id);
CREATE INDEX IF NOT EXISTS idx_business_transfers_status ON business_transfers(status);
CREATE INDEX IF NOT EXISTS idx_business_transfers_requested_by ON business_transfers(requested_by);

-- The money trail for an accepted transfer: what is payable, at which rate, and
-- whether it has settled. Kept separate from the transfer row so the fee status
-- can move (payable -> settled) without rewriting the ownership audit record.
CREATE TABLE IF NOT EXISTS business_transfer_fees (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    transfer_id    uuid NOT NULL REFERENCES business_transfers(id) ON DELETE CASCADE,
    business_id    uuid REFERENCES businesses(id) ON DELETE SET NULL,
    payer_user_id  uuid,
    payee_user_id  uuid,
    amount_cents   integer NOT NULL DEFAULT 0,
    currency       varchar(8) NOT NULL DEFAULT 'USD',
    status         varchar(20) NOT NULL DEFAULT 'payable',
    created_at     timestamptz NOT NULL DEFAULT now(),
    settled_at     timestamptz,
    CONSTRAINT business_transfer_fees_status_check
        CHECK (status IN ('payable', 'settled', 'waived')),
    CONSTRAINT business_transfer_fees_amount_check CHECK (amount_cents >= 0)
);

CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_transfer_id ON business_transfer_fees(transfer_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_business_id ON business_transfer_fees(business_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_payer_user_id ON business_transfer_fees(payer_user_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_payee_user_id ON business_transfer_fees(payee_user_id);
CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_status ON business_transfer_fees(status);
