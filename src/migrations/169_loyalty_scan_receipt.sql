-- 169_loyalty_scan_receipt.sql — record the receipt the counter scanner already collects
-- (item 18 follow-on: a key the front-end sends and the server silently discarded).
--
-- The browser scanner (frontend/scanner.html) generates a fresh transaction id for every
-- scan (`freshTxId()`), shows it to staff read-only as the receipt number, and POSTs it to
-- `/api/v1/loyalty/scan` as `transaction_id` — alongside the free-text `notes` and the
-- business `category`. `CounterScanInput` never had those fields, so every one was dropped on
-- the floor: the ledger row could not be tied back to the receipt a customer holds, and a
-- resubmitted/retried scan credited the member a SECOND time (nothing linked the two scans).
--
-- This adds the two missing columns (`business_category` already exists on the table) and a
-- partial unique index so the same receipt can never be recorded twice at one business.
-- The index is partial (WHERE transaction_id IS NOT NULL) so the 1.5k+ historical rows and
-- every clearinghouse-settled row (which carries no counter receipt) are unaffected.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB (src/db.rs re-applies
-- every file in src/migrations on each boot).

ALTER TABLE public.loyalty_scans ADD COLUMN IF NOT EXISTS transaction_id text;
ALTER TABLE public.loyalty_scans ADD COLUMN IF NOT EXISTS notes text;

CREATE UNIQUE INDEX IF NOT EXISTS uq_loyalty_scans_business_transaction
    ON public.loyalty_scans (business_id, transaction_id)
    WHERE transaction_id IS NOT NULL;
