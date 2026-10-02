-- 110 retire the "Connect IncentiveSwift" flow (David's decision 2026-09-23:
--     "we can retire connect incentives with to multi-directory").
--
-- Multi-Directory's IS connect flow accepted an IncentiveSwift API key for the signed-in
-- user's directory. It was a duplicate of the FunnelSwift connector model and an unbounded
-- trust surface. David's binding rules now say: loyalty is NATIVE Multi-Directory code
-- (ZaarCash, tables/routes in loyalty_native.rs + the native clearinghouse) and CoreSwift
-- CRM is the ONLY external integration. The connect UI, the IS verify call, the IS arms of
-- connect/verify/disconnect and the IS campaigns proxy are all gone from the code.
--
-- This migration finishes the job at the storage layer:
--   1. delete any stored IncentiveSwift connection row (0 rows measured 2026-10-02, but the
--      statement is what makes "no stored IS key" true regardless of history);
--   2. narrow the service CHECK constraint to the one service that still exists, so a future
--      writer cannot re-introduce an IncentiveSwift connection behind the code's back.
--
-- NOT VALID then VALIDATE, so the constraint can never fail on a row the DELETE missed.

DELETE FROM connected_services WHERE service = 'incentiveswift';

ALTER TABLE connected_services DROP CONSTRAINT IF EXISTS connected_services_service_check;

ALTER TABLE connected_services ADD CONSTRAINT connected_services_service_check
  CHECK (service = 'coreswift') NOT VALID;

ALTER TABLE connected_services VALIDATE CONSTRAINT connected_services_service_check;
