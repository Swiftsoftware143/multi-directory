-- Portfolio/customer separation — the marker on the ACCOUNT side (kanban t_9479f495)
--
-- The owner's own companies are `directories` rows owned by a platform-tenant user; the Customers
-- list is `tenants`. Separation therefore holds only as long as no portfolio company is ever given
-- an account of its own — because that account is necessarily a `tenants` row, and the customer-list
-- query (`WHERE t.id <> <platform>`) would hand it straight back to the Customers list. Proven on a
-- copy of the live database before this migration existed: a tenant linked as a directory's own
-- account came back with `has_account: true` AND appeared in the `tenants` array.
--
-- Two additive columns, nothing dropped and nothing renamed:
--
-- 1. `tenants.is_portfolio` — the marker. Every existing tenant is a customer, so the default is
--    false and the backfill below can only ever SET the flag on a tenant that a portfolio directory
--    already names as its own account.
-- 2. `directories.account_tenant_id` — the portfolio row's OWN account, nullable (NULL is what lets
--    a company exist with nothing behind it, so the screen can say "No account"). It is deliberately
--    NOT `coreswift_tenant_id`: on this install all ten of the owner's companies share one
--    `coreswift_tenant_id` that is a CoreSwift workspace id, not a tenant of this app, and it is
--    what the booking/CoreSwift integration dials. Writing an account id into it would break that
--    link — so the account reference gets its own column.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS is_portfolio boolean NOT NULL DEFAULT false;

ALTER TABLE directories
    ADD COLUMN IF NOT EXISTS account_tenant_id uuid REFERENCES tenants(id) ON DELETE SET NULL;

-- Backfill (idempotent, and a no-op on an install that has no portfolio accounts yet): a tenant is a
-- portfolio account by definition when a directory that is owned by a platform-tenant user names it
-- as its OWN account. The platform tenant itself is excluded — it is the operator's account, not a
-- company's, and the customer list already excludes it by id.
UPDATE tenants t
   SET is_portfolio = true
 WHERE t.is_portfolio = false
   AND t.id <> '00000000-0000-0000-0000-000000000001'
   AND (EXISTS (
         SELECT 1
           FROM directories d
           JOIN users u ON u.id = d.owner_id
          WHERE d.account_tenant_id = t.id
            AND u.tenant_id = '00000000-0000-0000-0000-000000000001'
       )
       OR EXISTS (
         SELECT 1
           FROM directories d
           JOIN users u ON u.id = d.owner_id
          WHERE d.coreswift_tenant_id = t.id
            AND u.tenant_id = '00000000-0000-0000-0000-000000000001'
       ));
