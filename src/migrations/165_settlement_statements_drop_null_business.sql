-- 165_settlement_statements_drop_null_business.sql
--
-- Defect found while TEST+FIXing the loyalty payouts/settlement card (B185):
-- the `settlement_statements` view pools a run's invoices and payouts by business_id
-- through `CROSS JOIN LATERAL (… UNION …)`. When an invoice or payout carries a NULL
-- business_id (it was raised against a business that no longer resolves in `businesses`),
-- that NULL formed its OWN group, so the run emitted one extra blank statement:
-- business_name NULL, points 0 and every amount 0.00. It was rendered as an empty row in
-- the admin Settlement card and appeared in the statements.csv export.
--
-- A statement with no business is meaningless — exclude unattributed rows from the view.
-- The run totals (`settlement_runs.total_invoiced_cents` / `total_payout_cents`) are
-- unaffected; they are aggregated at run time, not from this view.
--
-- CREATE OR REPLACE keeps this idempotent (the view is also created by the 000 baseline,
-- so this file must never be edited once applied — a further change is a new number).

CREATE OR REPLACE VIEW public.settlement_statements AS
 SELECT r.id AS run_id,
    r.network_id,
    r.period_key,
    r.period_start,
    r.period_end,
    r.status AS run_status,
    r.currency,
    b.business_id,
    COALESCE(i.business_name, p.business_name) AS business_name,
    COALESCE(i.points_issued, (0)::bigint) AS points_issued,
    COALESCE(i.amount_cents, (0)::numeric) AS invoiced_cents,
    COALESCE(p.points_redeemed, (0)::bigint) AS points_redeemed,
    COALESCE(p.amount_cents, (0)::numeric) AS reimbursed_cents,
    (COALESCE(i.amount_cents, (0)::numeric) - COALESCE(p.amount_cents, (0)::numeric)) AS net_position_cents,
    i.status AS invoice_status,
    p.status AS payout_status
   FROM (((public.settlement_runs r
     CROSS JOIN LATERAL ( SELECT settlement_invoices.business_id
           FROM public.settlement_invoices
          WHERE ((settlement_invoices.run_id = r.id) AND (settlement_invoices.business_id IS NOT NULL))
        UNION
         SELECT settlement_payouts.business_id
           FROM public.settlement_payouts
          WHERE ((settlement_payouts.run_id = r.id) AND (settlement_payouts.business_id IS NOT NULL))) b)
     LEFT JOIN public.settlement_invoices i ON (((i.run_id = r.id) AND (i.business_id = b.business_id))))
     LEFT JOIN public.settlement_payouts p ON (((p.run_id = r.id) AND (p.business_id = b.business_id))));
