-- Migration 087: clearinghouse settlement runs (T3) — close the money loop.
--
-- The loyalty ledger already works (point_issuance_log / point_redemption_log /
-- business_point_ledger hold live data) but the monthly settlement run that turns
-- that ledger into invoices, payouts and per-business statements was never built.
--
-- Economics are NOT hardcoded here: rate_issue_cents / rate_redeem_cents /
-- min_payout_cents / cycle_day / currency are copied onto each run from the
-- admin-editable settings row (point_treasury, extended below) at run time, so a
-- historical run keeps the rates it actually used.
--
-- Idempotency: UNIQUE (network_id, period_start, period_end) — re-running the same
-- period can never double-bill; the existing run is returned instead.

CREATE TABLE IF NOT EXISTS settlement_runs (
    id                    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id            uuid NOT NULL,
    period_start          date NOT NULL,
    period_end            date NOT NULL,
    period_key            varchar(16) NOT NULL,
    status                varchar(24) NOT NULL DEFAULT 'draft',
    currency              varchar(8) NOT NULL DEFAULT 'USD',
    -- Rates are DOLLARS PER POINT (matching point_treasury.issuance_rate = 0.01 and
    -- redemption_rate = 0.008), NOT cents. Named explicitly so nobody multiplies by
    -- 100 twice: amount_cents = points * rate_per_point * 100.
    rate_issue_per_point  numeric(12,6) NOT NULL DEFAULT 0.010000,
    rate_redeem_per_point numeric(12,6) NOT NULL DEFAULT 0.008000,
    min_payout_cents      integer NOT NULL DEFAULT 0,
    cycle_day             integer NOT NULL DEFAULT 1,
    total_points_issued   bigint NOT NULL DEFAULT 0,
    total_points_redeemed bigint NOT NULL DEFAULT 0,
    total_invoiced_cents  numeric(14,2) NOT NULL DEFAULT 0,
    total_payout_cents    numeric(14,2) NOT NULL DEFAULT 0,
    platform_spread_cents numeric(14,2) NOT NULL DEFAULT 0,
    provider              varchar(32),
    provider_ref          text,
    provider_message      text,
    created_by            uuid,
    created_at            timestamptz NOT NULL DEFAULT now(),
    completed_at          timestamptz,
    notes                 text,
    CONSTRAINT settlement_runs_status_check CHECK (status IN (
        'draft', 'preview', 'pending_provider', 'processing', 'completed', 'failed', 'cancelled'
    )),
    CONSTRAINT settlement_runs_network_period_key UNIQUE (network_id, period_start, period_end)
);

CREATE INDEX IF NOT EXISTS idx_settlement_runs_network_id ON settlement_runs(network_id);
CREATE INDEX IF NOT EXISTS idx_settlement_runs_status ON settlement_runs(status);
CREATE INDEX IF NOT EXISTS idx_settlement_runs_period_key ON settlement_runs(period_key);
CREATE INDEX IF NOT EXISTS idx_settlement_runs_created_by ON settlement_runs(created_by);

-- What each issuing business owes for points it issued in the period.
CREATE TABLE IF NOT EXISTS settlement_invoices (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id        uuid NOT NULL REFERENCES settlement_runs(id) ON DELETE CASCADE,
    business_id   uuid REFERENCES businesses(id) ON DELETE SET NULL,
    business_name text,
    points_issued bigint NOT NULL DEFAULT 0,
    rate_per_point numeric(12,6) NOT NULL DEFAULT 0.010000,
    amount_cents  numeric(14,2) NOT NULL DEFAULT 0,
    currency      varchar(8) NOT NULL DEFAULT 'USD',
    status        varchar(24) NOT NULL DEFAULT 'pending',
    provider_ref  text,
    due_date      date,
    created_at    timestamptz NOT NULL DEFAULT now(),
    settled_at    timestamptz,
    CONSTRAINT settlement_invoices_status_check CHECK (status IN (
        'pending', 'pending_provider', 'sent', 'paid', 'failed', 'void', 'below_minimum'
    )),
    CONSTRAINT settlement_invoices_run_business_key UNIQUE (run_id, business_id)
);

CREATE INDEX IF NOT EXISTS idx_settlement_invoices_run_id ON settlement_invoices(run_id);
CREATE INDEX IF NOT EXISTS idx_settlement_invoices_business_id ON settlement_invoices(business_id);
CREATE INDEX IF NOT EXISTS idx_settlement_invoices_status ON settlement_invoices(status);

-- What each redeeming business is reimbursed for points it redeemed in the period.
CREATE TABLE IF NOT EXISTS settlement_payouts (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          uuid NOT NULL REFERENCES settlement_runs(id) ON DELETE CASCADE,
    business_id     uuid REFERENCES businesses(id) ON DELETE SET NULL,
    business_name   text,
    points_redeemed bigint NOT NULL DEFAULT 0,
    rate_per_point numeric(12,6) NOT NULL DEFAULT 0.008000,
    amount_cents    numeric(14,2) NOT NULL DEFAULT 0,
    currency        varchar(8) NOT NULL DEFAULT 'USD',
    status          varchar(24) NOT NULL DEFAULT 'pending',
    provider        varchar(32),
    provider_ref    text,
    provider_message text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    paid_at         timestamptz,
    CONSTRAINT settlement_payouts_status_check CHECK (status IN (
        'pending', 'pending_provider', 'paid', 'failed', 'void', 'below_minimum'
    )),
    CONSTRAINT settlement_payouts_run_business_key UNIQUE (run_id, business_id)
);

CREATE INDEX IF NOT EXISTS idx_settlement_payouts_run_id ON settlement_payouts(run_id);
CREATE INDEX IF NOT EXISTS idx_settlement_payouts_business_id ON settlement_payouts(business_id);
CREATE INDEX IF NOT EXISTS idx_settlement_payouts_status ON settlement_payouts(status);

-- Per-business statement view: issued side, redeemed side and net position, so a
-- statement is one SELECT and the CSV export is one query.
-- The business set is the UNION of invoice and payout businesses (a business can
-- legitimately appear on only one side), then both sides are LEFT JOINed on — a
-- plain FULL JOIN chain would drop the run context for payout-only businesses.
CREATE OR REPLACE VIEW settlement_statements AS
SELECT
    r.id            AS run_id,
    r.network_id    AS network_id,
    r.period_key    AS period_key,
    r.period_start  AS period_start,
    r.period_end    AS period_end,
    r.status        AS run_status,
    r.currency      AS currency,
    b.business_id   AS business_id,
    COALESCE(i.business_name, p.business_name)                  AS business_name,
    COALESCE(i.points_issued, 0)                                AS points_issued,
    COALESCE(i.amount_cents, 0)                                 AS invoiced_cents,
    COALESCE(p.points_redeemed, 0)                              AS points_redeemed,
    COALESCE(p.amount_cents, 0)                                 AS reimbursed_cents,
    (COALESCE(i.amount_cents, 0) - COALESCE(p.amount_cents, 0)) AS net_position_cents,
    i.status        AS invoice_status,
    p.status        AS payout_status
FROM settlement_runs r
CROSS JOIN LATERAL (
    SELECT business_id FROM settlement_invoices WHERE run_id = r.id
    UNION
    SELECT business_id FROM settlement_payouts  WHERE run_id = r.id
) b
LEFT JOIN settlement_invoices i ON i.run_id = r.id AND i.business_id = b.business_id
LEFT JOIN settlement_payouts  p ON p.run_id = r.id AND p.business_id = b.business_id;

-- Admin-editable settlement settings live on the existing per-network treasury row
-- (it already carries issuance_rate / redemption_rate / default_expiry_days).
-- New columns only; no data is rewritten.
ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS cycle_day integer NOT NULL DEFAULT 1;
ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS currency varchar(8) NOT NULL DEFAULT 'USD';
ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS minimum_payout_cents integer NOT NULL DEFAULT 0;
ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS settlement_enabled boolean NOT NULL DEFAULT true;
-- Which payment provider moves the money. NULL = auto-detect the first configured
-- provider from provider_keys. Never hardcoded: the admin picks it in the panel, and
-- with nothing configured the settlement run is marked pending_provider instead of
-- pretending money moved.
ALTER TABLE point_treasury ADD COLUMN IF NOT EXISTS payment_provider varchar(32);
