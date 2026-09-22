-- 100: ONE source of truth for business_type (card B53).
--
-- The DB CHECK constraint, the register handler's accepted list and the register
-- dropdowns each carried a different set of values. Live consequences:
--   * `other` was accepted by the handler, rejected here -> HTTP 500 AFTER the
--     visitor_accounts row was committed (orphaned account, no business);
--   * `supplier`, `local` and `chain` are accepted here but the handler refused them,
--     so REGISTERING AS A SUPPLIER WAS IMPOSSIBLE;
--   * `manufacturer` was missing from the supplier query lists, so a manufacturer
--     account registered 201 and then 404'd on every supplier endpoint.
--
-- This migration makes the constraint equal to `src/business_types.rs::BUSINESS_TYPES`
-- (migration 074 built the previous, shorter list; 'other' is the only addition). The
-- handler validates against that same constant and `GET /api/v1/b2b/business-types`
-- serves it to the UI, so the three can no longer drift apart.
--
-- Idempotent: DROP IF EXISTS + ADD, and the ADD is guarded so a re-run is a no-op
-- even if the constraint definition text ever differs in formatting.

ALTER TABLE businesses DROP CONSTRAINT IF EXISTS businesses_business_type_check;

ALTER TABLE businesses
    ADD CONSTRAINT businesses_business_type_check
    CHECK (business_type = ANY (ARRAY[
        'local'::text,
        'supplier'::text,
        'distributor'::text,
        'wholesaler'::text,
        'farm'::text,
        'association'::text,
        'manufacturer'::text,
        'chain'::text,
        'other'::text
    ]));

COMMENT ON COLUMN businesses.business_type IS
    'Business taxonomy. Canonical list lives in src/business_types.rs::BUSINESS_TYPES; this CONSTRAINT, the /api/v1/b2b/register validation and the register UI all read from it.';
