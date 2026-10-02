-- 071_correct_zaarhub_cities: Replace the incorrect 20-city seed with the
-- 10 actual Florida cities that ZaarHub operates in.
--
-- The dropdown on zaarhub.com is fed by zaarhub_cities::list_cities, which
-- reads city_pages WHERE is_active = true. Prior seed data contained 20
-- out-of-state/extra cities. This migration deactivates those and inserts the
-- correct 10 Florida cities as active.

-- Deactivate the previous seed so it no longer appears in the dropdown.
UPDATE city_pages SET is_active = false;

-- Insert the 10 correct Florida cities (idempotent on tenant+slug unique index).
INSERT INTO city_pages (tenant_id, city_slug, city_name, state, is_active, display_order)
VALUES
    ('00000000-0000-0000-0000-000000000001', 'palm-bay',       'Palm Bay',       'FL', true, 1),
    ('00000000-0000-0000-0000-000000000001', 'st-cloud',       'St. Cloud',      'FL', true, 2),
    ('00000000-0000-0000-0000-000000000001', 'winter-garden',  'Winter Garden',  'FL', true, 3),
    ('00000000-0000-0000-0000-000000000001', 'st-petersburg',  'St. Petersburg', 'FL', true, 4),
    ('00000000-0000-0000-0000-000000000001', 'apopka',         'Apopka',         'FL', true, 5),
    ('00000000-0000-0000-0000-000000000001', 'lake-nona',      'Lake Nona',      'FL', true, 6),
    ('00000000-0000-0000-0000-000000000001', 'hollywood',      'Hollywood',      'FL', true, 7),
    ('00000000-0000-0000-0000-000000000001', 'boca-raton',     'Boca Raton',     'FL', true, 8),
    ('00000000-0000-0000-0000-000000000001', 'pompano-beach',  'Pompano Beach',  'FL', true, 9),
    ('00000000-0000-0000-0000-000000000001', 'palm-coast',     'Palm Coast',     'FL', true, 10)
ON CONFLICT (tenant_id, city_slug)
DO UPDATE SET
    city_name = EXCLUDED.city_name,
    state = EXCLUDED.state,
    is_active = true,
    display_order = EXCLUDED.display_order,
    updated_at = now();

-- Mirror the correct cities into the directories table so the homepage's
-- network_visible dropdown/source-of-truth stays in sync.
INSERT INTO directories (name, slug, description, status, city, zaarhub_config)
VALUES
    ('Palm Bay',       'palm-bay',       'Palm Bay, Florida',       'active', 'Palm Bay, FL',       '{"network_visible": true}'::jsonb),
    ('St. Cloud',      'st-cloud',       'St. Cloud, Florida',      'active', 'St. Cloud, FL',      '{"network_visible": true}'::jsonb),
    ('Winter Garden',  'winter-garden',  'Winter Garden, Florida',  'active', 'Winter Garden, FL',  '{"network_visible": true}'::jsonb),
    ('St. Petersburg', 'st-petersburg',  'St. Petersburg, Florida', 'active', 'St. Petersburg, FL', '{"network_visible": true}'::jsonb),
    ('Apopka',         'apopka',         'Apopka, Florida',         'active', 'Apopka, FL',         '{"network_visible": true}'::jsonb),
    ('Lake Nona',      'lake-nona',      'Lake Nona, Florida',      'active', 'Lake Nona, FL',      '{"network_visible": true}'::jsonb),
    ('Hollywood',      'hollywood',      'Hollywood, Florida',      'active', 'Hollywood, FL',      '{"network_visible": true}'::jsonb),
    ('Boca Raton',     'boca-raton',     'Boca Raton, Florida',     'active', 'Boca Raton, FL',     '{"network_visible": true}'::jsonb),
    ('Pompano Beach',  'pompano-beach',  'Pompano Beach, Florida',  'active', 'Pompano Beach, FL',  '{"network_visible": true}'::jsonb),
    ('Palm Coast',     'palm-coast',     'Palm Coast, Florida',     'active', 'Palm Coast, FL',     '{"network_visible": true}'::jsonb)
ON CONFLICT (slug)
DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    status = 'active',
    city = EXCLUDED.city,
    zaarhub_config = EXCLUDED.zaarhub_config,
    updated_at = now();

-- Hide the out-of-state/extra directories from the network.
UPDATE directories
SET zaarhub_config = COALESCE(zaarhub_config, '{}'::jsonb) || '{"network_visible": false}'::jsonb
WHERE slug NOT IN (
    'palm-bay','st-cloud','winter-garden','st-petersburg','apopka','lake-nona',
    'hollywood','boca-raton','pompano-beach','palm-coast'
);
