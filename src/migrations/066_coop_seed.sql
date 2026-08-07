-- Migration 066: Co-op seed data
INSERT INTO buying_groups (name, description, category, founder_business_id, status, min_members, max_members)
SELECT
    'Organic Produce Co-op',
    'A group of restaurants pooling purchasing power to get wholesale prices on organic produce. Join us to save 30%+ on your weekly supply.',
    'produce',
    b.id,
    'recruiting',
    5,
    20
FROM businesses b
WHERE b.business_type IN ('distributor', 'farm', 'association', 'supplier')
  AND b.is_active = COALESCE(b.is_active, true)
  AND NOT EXISTS (SELECT 1 FROM buying_groups LIMIT 1)
LIMIT 1;
