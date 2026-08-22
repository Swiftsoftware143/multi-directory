-- Migration 081: Seed default redemption caps into the ZaarHub network.
-- Migration 079's cap seed ran before the network row existed, so it inserted
-- nothing. This backfills the standard caps now that the network is present.
INSERT INTO category_redeem_caps (network_id, category_name, max_redeem_percent, description)
SELECT n.id, c.category_name, c.max_redeem_percent, c.description
FROM networks n
CROSS JOIN (VALUES
    ('grocery', 20, 'Low-margin: points cap at 20% of invoice'),
    ('convenience', 20, 'Low-margin convenience: cap at 20% of invoice'),
    ('fuel', 20, 'Fuel: cap at 20% of invoice'),
    ('tobacco', 0, 'Disallowed: no points redemption'),
    ('restaurant', 100, 'Full points redemption allowed'),
    ('retail', 100, 'Full points redemption allowed'),
    ('services', 100, 'Full points redemption allowed')
) AS c(category_name, max_redeem_percent, description)
WHERE NOT EXISTS (
    SELECT 1 FROM category_redeem_caps cr
    WHERE cr.network_id = n.id AND cr.category_name = c.category_name
);
