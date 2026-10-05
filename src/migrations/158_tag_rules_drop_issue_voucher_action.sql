-- Retire the 'issue_voucher' tag-rule action type.
--
-- The action's only implementation was src/handlers/tag_automation.rs::
-- execute_voucher_action(), which POSTed to IncentiveSwift's local
-- http://localhost:8083/api/v1/loyalty/issue-voucher. That route was retired at
-- source (IncentiveSwift commit 43d3ce8a) because loyalty is native to
-- multi-directory; MD's own tag-rule form (frontend/admin-ops.html) never
-- offered issue_voucher, and tag_rules has never held a single row.
--
-- The caller is deleted, so the permitted value can no longer mean anything.
-- Measured on live before writing: tag_rules holds 0 rows (and 0 carrying
-- issue_voucher), so re-creating the constraint cannot fail on existing data.
-- One statement, so the swap is atomic: the table is never left without the
-- constraint. Re-runnable (DROP IF EXISTS) if the ledger retries this file.

ALTER TABLE tag_rules
    DROP CONSTRAINT IF EXISTS tag_rules_action_type_check,
    ADD CONSTRAINT tag_rules_action_type_check
        CHECK (action_type = ANY (ARRAY[
            'send_email'::text,
            'send_sms'::text,
            'webhook'::text,
            'pipeline_move'::text,
            'scoring_update'::text,
            'add_tag'::text,
            'remove_tag'::text
        ]));
