-- 217_mcp_reader_newsletter_write.sql
--
-- The fleet MCP (:8099) now exposes WRITE tools for the newsletter config
-- (multi_directory_newsletter_set_registration / upload_image), so the CEO can manage per-city
-- newsletter links and adverts without a code change. The write is scoped to the caller's
-- directories by the MCP server itself; this migration only grants the narrow DB permission the
-- `mcp_reader` role needs to carry it out:
--
--   * UPDATE on the single `newsletter_registration_url` column (column-level, least privilege —
--     `mcp_reader` still cannot change any other column of `directories`); and
--   * INSERT on `directory_events`, because the `directories` UPDATE trigger
--     (`trg_auto_record_event`) records an audit row on every change.
--
-- `mcp_reader` remains otherwise SELECT-only. Grant is idempotent.

GRANT UPDATE (newsletter_registration_url) ON directories TO mcp_reader;
GRANT INSERT ON directory_events TO mcp_reader;
