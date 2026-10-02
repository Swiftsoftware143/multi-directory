-- zaarhub_site_config never had a copyright_year column, yet the admin console exposes a
-- "Copyright Years" field (frontend/zaarhub-admin.html #cfg-copyright_year) and
-- src/handlers/zaarhub_admin.rs reads it on GET, and writes it on both the INSERT and the
-- UPDATE path. Every site-config save therefore failed with ERROR 42703 (column does not
-- exist) and the GET always answered an empty copyright_year.
ALTER TABLE zaarhub_site_config ADD COLUMN IF NOT EXISTS copyright_year VARCHAR(32);
