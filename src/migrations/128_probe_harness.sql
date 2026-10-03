-- 128_probe_harness.sql — fleet probe-residue policy convention (c): harness attribution marker.
--
-- Policy: /opt/swift/docs/fleet-probe-residue-policy-2026-09-28.md (kanban t_3c7f8623, t_d5e5af7e).
--
-- A row created by an automated test harness through a REAL product route may be marked with the
-- `X-Swift-Harness` request header. The marker is written at CREATION time, from the header ONLY
-- (never from the body or a query field), and is NULL for every other create — so a row created by
-- a real customer is byte-identical to what it was before this column existed.
--
-- MEASURED on live 2026-10-02 (audits/t_bedfa8ce/, audits/t_3c7f8623/): a headless-Chrome
-- supplier-portal run minted, in ONE `b2b_register` transaction, 2 `businesses` + 2
-- `visitor_accounts` + 2 `claimed_businesses` rows that were indistinguishable from a real
-- supplier's — the claim row alone lights the public "Verified Owner" badge. The 19 anonymous
-- `visitors` rows carry no attribution at all.
--
-- Scope: the four tables the supplier-registration / visitor-tracking paths write. No backfill —
-- every existing row stays NULL (never authorised, never falsely attributed).
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

ALTER TABLE visitors         ADD COLUMN IF NOT EXISTS probe_harness text NULL;
ALTER TABLE businesses       ADD COLUMN IF NOT EXISTS probe_harness text NULL;
ALTER TABLE claimed_businesses ADD COLUMN IF NOT EXISTS probe_harness text NULL;
ALTER TABLE visitor_accounts ADD COLUMN IF NOT EXISTS probe_harness text NULL;

COMMENT ON COLUMN visitors.probe_harness IS
  'Fleet probe-residue marker (convention (c)): value of the X-Swift-Harness request header when '
  'the row was created by an automated harness, NULL for every real visitor. Never backfilled.';

COMMENT ON COLUMN businesses.probe_harness IS
  'Fleet probe-residue marker (convention (c)): value of the X-Swift-Harness request header when '
  'the business was created by an automated harness (e.g. POST /api/v1/b2b/register), NULL for '
  'every real business. Never backfilled.';

COMMENT ON COLUMN claimed_businesses.probe_harness IS
  'Fleet probe-residue marker (convention (c)): value of the X-Swift-Harness request header when '
  'the claim was created by an automated harness, NULL for every real claim. A claim row alone '
  'lights the public "Verified Owner" badge, so this is the column the sweeper attributes it by.';

COMMENT ON COLUMN visitor_accounts.probe_harness IS
  'Fleet probe-residue marker (convention (c)): value of the X-Swift-Harness request header when '
  'the account was created by an automated harness (e.g. POST /api/v1/b2b/register), NULL for '
  'every real account. Never backfilled.';

CREATE INDEX IF NOT EXISTS idx_visitors_probe_harness          ON visitors (probe_harness)          WHERE probe_harness IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_businesses_probe_harness        ON businesses (probe_harness)        WHERE probe_harness IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_claimed_businesses_probe_harness ON claimed_businesses (probe_harness) WHERE probe_harness IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_visitor_accounts_probe_harness  ON visitor_accounts (probe_harness)  WHERE probe_harness IS NOT NULL;
