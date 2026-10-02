-- 109: give the payment-webhook refusal arms their own audit status.
--
-- `payment_webhook_events.status` admitted only received|processed|failed|ignored, so the Stripe
-- receiver's two refusal arms had to be written as 'failed' and could not be told apart: "no active
-- stripe row / no signing secret" (an operator fixes it in Admin > Payment gateways and the queued
-- Stripe retries then verify) looked identical to "the signature did not verify" (the secret is
-- present but the delivery's HMAC did not match ours). The receiver now writes
--
--   not_configured  — no active 'stripe' row, or one with no signing secret stored
--   signature_failed — a secret IS stored and the signature is absent or does not verify
--
-- and keeps `error_message` carrying the exact response reason. Same shape as WorkflowSwift's
-- migration 060 (kanban t_40b77d6a); the arm contract here is t_10d3ad0c.
--
-- DROP + ADD on a text CHECK: no column change, no row rewritten, no data touched, so it is safe to
-- apply on a live table (the receiver's next INSERT is the only thing that needs the wider set).

ALTER TABLE payment_webhook_events
    DROP CONSTRAINT IF EXISTS payment_webhook_events_status_check;

ALTER TABLE payment_webhook_events
    ADD CONSTRAINT payment_webhook_events_status_check
    CHECK (status = ANY (ARRAY['received', 'processed', 'failed', 'ignored',
                               'not_configured', 'signature_failed']));
