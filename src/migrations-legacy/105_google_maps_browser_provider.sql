-- 105 — Google Maps (browser) provider + data-driven BROWSER-ONLY labelling.
--
-- WHY: a Google map on a business page is drawn BY THE VISITOR'S BROWSER, so it needs the
-- Maps JavaScript API key — a DIFFERENT credential from the server-side Places API key that
-- drives search/enrichment. Until now there was no provider row a browser key could be stored
-- under, so POST /api/v1/provider-keys {provider:"google_maps"} returned 404 and the browser
-- key had nowhere to live.
--
-- `browser_only` is a property of the PROVIDER (it travels to the visitor's browser), not of a
-- particular key, so it belongs on the catalogue. The admin panel, the business portal and the
-- public maps-config reader all read this column, which is what makes the BROWSER-ONLY label
-- data-driven instead of a hardcoded special case.

ALTER TABLE available_providers
    ADD COLUMN IF NOT EXISTS browser_only BOOLEAN NOT NULL DEFAULT false;

INSERT INTO available_providers
    (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only)
VALUES
    ('google_maps',
     'Google Maps (browser)',
     'Draws the interactive map on a business page, in the visitor''s browser. BROWSER-ONLY: the key is visible in the page source, so restrict it by HTTP referrer to your own domains. It cannot do server-side search — that is the Google Places key.',
     false,
     '[]'::jsonb,
     '🗺',
     'Google Maps JavaScript API key',
     'BROWSER-ONLY — travels to the visitor''s browser. In Google Cloud restrict this key by HTTP referrer to your own domains (e.g. https://zaarhub.com/*). Never restrict it by server IP, never use it for server-side calls, and never reuse one "master" key for both jobs.',
     true)
ON CONFLICT (key) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    requires_base_url = EXCLUDED.requires_base_url,
    requires_metadata = EXCLUDED.requires_metadata,
    icon = EXCLUDED.icon,
    field_label = EXCLUDED.field_label,
    field_help = EXCLUDED.field_help,
    browser_only = EXCLUDED.browser_only;

-- The SHORT version of "which Google key goes where", stored with the provider row so both the
-- admin panel and the business portal show it next to the field the admin actually pastes into.
-- (The admin guide carries the long version.)
UPDATE available_providers
SET field_help = 'SERVER-SIDE ONLY — business search, discovery and enrichment. Restrict this key in Google Cloud by server IP address; never by website referrer. Already set at network level, so a city directory with no key of its own inherits it.',
    browser_only = false
WHERE key = 'google_places';
