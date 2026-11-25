-- 218_revoke_mcp_reader_write.sql
--
-- Revokes the two write grants 217 originally gave `mcp_reader`:
--   * UPDATE (newsletter_registration_url) on `directories` (column-level); and
--   * INSERT on `directory_events` (the audit rows the `directories` UPDATE trigger writes).
--
-- The only consumers were the fleet MCP's two Multi-Directory newsletter write tools, which were
-- deleted (fleet-mcp commit 6ead3bc). `mcp_reader` is SELECT-only by the fleet invariant
-- ("no DB-direct write, ever"), so these grants are dead privilege.
--
-- Idempotent and safe to re-run: REVOKE of a privilege that is not granted is a no-op, and the
-- role-existence guard makes the whole file a no-op on a fresh install that has never provisioned
-- the `mcp_reader` role (that role is created out-of-band by the fleet MCP, not by this app's
-- migrations).

DO $rev$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'mcp_reader') THEN
    EXECUTE 'REVOKE UPDATE (newsletter_registration_url) ON directories FROM mcp_reader';
    EXECUTE 'REVOKE INSERT ON directory_events FROM mcp_reader';
  END IF;
END $rev$;
