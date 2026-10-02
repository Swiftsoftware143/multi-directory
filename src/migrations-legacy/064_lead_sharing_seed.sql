-- Migration 064: Lead sharing seed data
INSERT INTO shared_leads (title, description, category, location, estimated_value, source, poster_business_id, status, expires_at)
SELECT
    'Commercial Kitchen Equipment — Bulk Order',
    'A client needs 50 commercial-grade refrigerators for a new hotel chain. Our manufacturing capacity is full. Looking for a partner to co-supply.',
    'equipment',
    'Southeast US',
    125000.00,
    'overflow',
    b.id,
    'available',
    NOW() + INTERVAL '30 days'
FROM businesses b
WHERE b.business_type IN ('supplier', 'distributor', 'manufacturer')
  AND b.is_active = COALESCE(b.is_active, true)
  AND NOT EXISTS (SELECT 1 FROM shared_leads LIMIT 1)
LIMIT 1;
