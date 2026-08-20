-- Migration 077: register CoreSwift as an available integration provider
-- Enables per-directory CoreSwift connection via Integration Center (provider_keys),
-- which the native loyalty engine uses for the CRM drill-down.
INSERT INTO available_providers (key, name, description, requires_base_url, requires_metadata, icon)
VALUES (
    'coreswift',
    'CoreSwift CRM',
    'Drill loyalty members and business participants into your CoreSwift CRM. Connect with a personal API key (csk_...) from CoreSwift Integration Center.',
    true,
    '[]'::jsonb,
    'crm'
)
ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description;
