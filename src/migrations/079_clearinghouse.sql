-- Migration 077: Clearinghouse economics + network-wide loyalty wallet
-- Brings the IncentiveSwift clearinghouse natively into Multi-Directory,
-- re-keyed to NETWORK scope so ZaarCash is a single universal wallet across
-- all city directories (earn in Palm Bay, redeem in St. Pete).
--
-- Model:
--   loyalty_programs.network_id (nullable): when set, the program is a
--     network-wide program shared by all directories in that network.
--   A member's wallet is keyed to (network_program, visitor_account_id),
--     so points carry across every city in the network.
--   Clearinghouse tables are network-scoped.

-- ============================================================
-- 1. Network-wide loyalty program scoping
-- ============================================================
ALTER TABLE loyalty_programs ADD COLUMN IF NOT EXISTS network_id uuid REFERENCES networks(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_network ON loyalty_programs(network_id);

-- Make member lookup unique per (program, visitor) but allow a program to be
-- network-wide. Add network_id to members so the one-wallet-per-visitor constraint
-- is a simple partial index (no subquery allowed in partial index predicates).
ALTER TABLE loyalty_members ADD COLUMN IF NOT EXISTS network_id uuid REFERENCES networks(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_loyalty_members_network_visitor
    ON loyalty_members (visitor_account_id)
    WHERE network_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_loyalty_members_network ON loyalty_members(network_id);

-- ============================================================
-- 2. Treasury (per network) — running totals + config
-- ============================================================
CREATE TABLE IF NOT EXISTS point_treasury (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id uuid REFERENCES networks(id) ON DELETE CASCADE,
    -- Economic config
    issuance_rate decimal(12,6) NOT NULL DEFAULT 0.010000,      -- $ per point billed to issuing business
    redemption_rate decimal(12,6) NOT NULL DEFAULT 0.008000,    -- $ per point reimbursed to redeeming business
    platform_spread_percent decimal(8,4) NOT NULL DEFAULT 20.0000,  -- % kept by platform (% of issuance rate)
    minimum_float decimal(14,2) NOT NULL DEFAULT 100.00,        -- minimum cash cushion
    default_expiry_days integer NOT NULL DEFAULT 365,           -- rolling 12-month expiration
    -- Running totals
    total_points_issued bigint NOT NULL DEFAULT 0,
    total_points_redeemed bigint NOT NULL DEFAULT 0,
    total_revenue_collected decimal(14,2) NOT NULL DEFAULT 0,
    total_reimbursements_paid decimal(14,2) NOT NULL DEFAULT 0,
    outstanding_liability decimal(14,2) NOT NULL DEFAULT 0,     -- points in circulation x issuance_rate
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (network_id)
);

-- ============================================================
-- 3. Business point ledger (per month) — net position per business
-- ============================================================
CREATE TABLE IF NOT EXISTS business_point_ledger (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id uuid REFERENCES networks(id) ON DELETE CASCADE,
    business_id uuid NOT NULL,                                   -- MD businesses(id)
    business_name text,
    month_key varchar(7) NOT NULL,                               -- 'YYYY-MM'
    points_issued_this_month bigint NOT NULL DEFAULT 0,
    points_redeemed_this_month bigint NOT NULL DEFAULT 0,
    total_billed_this_month decimal(14,2) NOT NULL DEFAULT 0,    -- points issued x issuance_rate
    total_reimbursed_this_month decimal(14,2) NOT NULL DEFAULT 0,-- points redeemed x redemption_rate
    net_position decimal(14,2) NOT NULL DEFAULT 0,               -- reimbursed - billed (+ = owes to business)
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (network_id, business_id, month_key)
);
CREATE INDEX IF NOT EXISTS idx_bpl_network_month ON business_point_ledger(network_id, month_key);

-- ============================================================
-- 4. Issuance log — business billed when it issues points
-- ============================================================
CREATE TABLE IF NOT EXISTS point_issuance_log (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id uuid REFERENCES networks(id) ON DELETE CASCADE,
    issuing_business_id uuid,
    business_name text,
    member_id uuid REFERENCES loyalty_members(id) ON DELETE SET NULL,
    program_id uuid REFERENCES loyalty_programs(id) ON DELETE SET NULL,
    scan_id uuid,
    points_issued integer NOT NULL DEFAULT 0,
    bill_rate_cents integer NOT NULL DEFAULT 1,                  -- $0.01 per point, in cents
    total_billed_cents integer NOT NULL DEFAULT 0,               -- points x rate
    transaction_amount decimal(14,2),
    transaction_id text,
    issuance_type text NOT NULL DEFAULT 'purchase',              -- purchase | checkin | visit
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_pil_business ON point_issuance_log(issuing_business_id);
CREATE INDEX IF NOT EXISTS idx_pil_member ON point_issuance_log(member_id);
CREATE INDEX IF NOT EXISTS idx_pil_network ON point_issuance_log(network_id, created_at DESC);

-- ============================================================
-- 5. Redemption log — redeeming business reimbursed
-- ============================================================
CREATE TABLE IF NOT EXISTS point_redemption_log (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id uuid REFERENCES networks(id) ON DELETE CASCADE,
    redeeming_business_id uuid,
    business_name text,
    member_id uuid REFERENCES loyalty_members(id) ON DELETE SET NULL,
    program_id uuid REFERENCES loyalty_programs(id) ON DELETE SET NULL,
    scan_id uuid,
    points_redeemed integer NOT NULL DEFAULT 0,
    reimbursement_rate_cents integer NOT NULL DEFAULT 0,         -- 0.8c per point
    total_reimbursement_cents integer NOT NULL DEFAULT 0,
    transaction_amount decimal(14,2),
    max_redeem_percent integer,                                  -- cap applied, e.g. 20
    transaction_id text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_prl_business ON point_redemption_log(redeeming_business_id);
CREATE INDEX IF NOT EXISTS idx_prl_member ON point_redemption_log(member_id);
CREATE INDEX IF NOT EXISTS idx_prl_network ON point_redemption_log(network_id, created_at DESC);

-- ============================================================
-- 6. Category redemption caps (guardrail vs. arbitrage)
-- ============================================================
CREATE TABLE IF NOT EXISTS category_redeem_caps (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id uuid REFERENCES networks(id) ON DELETE CASCADE,
    category_name varchar(100) NOT NULL,
    max_redeem_percent integer NOT NULL DEFAULT 100,             -- 20 for farms/grocery/low-margin
    description text,
    UNIQUE (network_id, category_name)
);
INSERT INTO category_redeem_caps (network_id, category_name, max_redeem_percent, description)
SELECT n.id, 'grocery', 20, 'Low-margin: points cap at 20% of invoice'
FROM networks n WHERE NOT EXISTS (SELECT 1 FROM category_redeem_caps WHERE category_name='grocery');

-- ============================================================
-- 7. Extend loyalty_scans with clearinghouse fields
-- ============================================================
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS business_name text;
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS points_balance integer;
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS deal_applied text;
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS transaction_amount decimal(14,2);
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS business_category text;
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS clearinghouse_processed boolean DEFAULT false;
ALTER TABLE loyalty_scans ADD COLUMN IF NOT EXISTS cleared_at timestamptz;


-- ============================================================
-- 9. ZaarHub network seed (idempotent)
--    Every city directory is assigned to the ZaarHub network so
--    loyalty is network-wide (earn in Palm Bay, redeem in St. Pete).
-- ============================================================
INSERT INTO networks (id, name, slug, description, status)
VALUES ('aaaaaaaa-0000-0000-0000-000000000001', 'ZaarHub', 'zaarhub',
        'Universal community loyalty network spanning all ZaarHub city directories', 'active')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
WHERE networks.slug = EXCLUDED.slug;

UPDATE directories
SET network_id = 'aaaaaaaa-0000-0000-0000-000000000001'
WHERE network_id IS NULL
  AND slug IN ('palm-bay','st-petersburg','apopka','boca-raton','hollywood',
               'lake-nona','palm-coast','pompano-beach','st-cloud','winter-garden');
