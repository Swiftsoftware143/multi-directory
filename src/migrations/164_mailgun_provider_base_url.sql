-- 164_mailgun_provider_base_url.sql
--
-- DEFECT: the Mailgun transport sends through the Mailgun HTTP API, which needs BOTH a private
-- API key AND the sending-domain URL (https://api.mailgun.net/v3/<domain>; EU region:
-- https://api.eu.mailgun.net/v3/<domain>). The in-house email service refuses a Mailgun send
-- without it: "mailgun needs its sending domain in the key's Base URL". But the provider
-- catalogue row for `mailgun` was seeded with requires_base_url = false and no field_label /
-- field_help, so the admin panel never rendered the sending-domain input on the Provider API
-- Keys card - the Mailgun path could not be configured from the UI at all (a capability that
-- exists only in code), and the only Mailgun config that could be saved was incomplete.
--
-- Fix the catalogue row so the card collects the sending domain with plain-English guidance.
-- The front end reads these columns straight from available_providers (admin-panel.html), so no
-- code change is required. Idempotent: safe on live and on a fresh install.
INSERT INTO available_providers
    (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only)
VALUES (
    'mailgun',
    'Mailgun',
    'Transactional + campaign email (Mailgun HTTP API)',
    true,
    '[]'::jsonb,
    '✉️',
    'Mailgun sending domain URL - e.g. https://api.mailgun.net/v3/updates.zaarhub.com',
    'Paste your Mailgun private API key in the value box. In this box enter your Mailgun sending domain URL: https://api.mailgun.net/v3/<your-domain> (EU region: https://api.eu.mailgun.net/v3/<your-domain>). Both are required - the key alone cannot send.',
    false
)
ON CONFLICT (key) DO UPDATE SET
    name               = EXCLUDED.name,
    description        = EXCLUDED.description,
    requires_base_url  = EXCLUDED.requires_base_url,
    requires_metadata  = EXCLUDED.requires_metadata,
    icon               = EXCLUDED.icon,
    field_label        = EXCLUDED.field_label,
    field_help         = EXCLUDED.field_help,
    browser_only       = EXCLUDED.browser_only;
