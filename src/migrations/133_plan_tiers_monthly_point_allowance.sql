-- 133_plan_tiers_monthly_point_allowance.sql — the per-tier monthly loyalty point allowance (card B113 / B114).
--
-- David (2026-10-02), on the self-serve pricing surface: the tier's monthly loyalty point ALLOWANCE
-- did not exist in the schema at all, so the pricing tab could show a price but not what the tier
-- actually includes, and the funding model had no field to size against.
--
-- SIZING RULE (from the funding model): an allowance is sized against the REIMBURSEMENT cost
-- ($0.008/point), never the face value ($0.01/point):
--   Starter  $19  -> 2,000 pts -> $16.00 cost  (funded)
--   Standard $49  -> 6,000 pts -> $48.00 cost  (funded)
--   Premium  $99  -> 12,000 pts -> $96.00 cost (funded)
-- Premium MUST be 12,000, not 15,000: 15,000 pts cost $120 to reimburse against a $99 price, i.e.
-- a guaranteed loss on every Premium subscriber. Never advertise a benefit the configuration cannot
-- pay for.
--
-- NULL means "not set" (no allowance configured) — the field is additive and changes no existing
-- behaviour; nothing consumes it yet.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS monthly_point_allowance integer;

COMMENT ON COLUMN plan_tiers.monthly_point_allowance IS
    'Points a business on this tier may issue per month; the subscription prepays them. Sized against the $0.008/point reimbursement cost, not the $0.01 face value. NULL = not configured.';

-- Seed the funded allowances once, keyed by the ZaarHub tier slugs and only where unset, so a later
-- admin edit is never overwritten by a re-run.
UPDATE plan_tiers
   SET monthly_point_allowance = CASE slug
           WHEN 'zaarhub-listed'   THEN 2000
           WHEN 'zaarhub-featured' THEN 6000
           WHEN 'zaarhub-premium'  THEN 12000
           ELSE monthly_point_allowance
       END
 WHERE monthly_point_allowance IS NULL;
