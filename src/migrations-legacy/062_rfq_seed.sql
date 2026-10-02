-- Migration 062: Seed sample RFQ data (optional; only if no RFQs exist)
INSERT INTO rfqs (title, description, category, quantity, budget_min, budget_max, deadline, delivery_location, poster_business_id, status, urgency, is_public)
SELECT
    'Fresh Organic Produce — Weekly Supply',
    'Looking for a reliable supplier of fresh organic vegetables and fruits for our restaurant chain. Need weekly deliveries to 3 locations.',
    'produce',
    'weekly ongoing',
    2000.00,
    5000.00,
    CURRENT_DATE + INTERVAL '14 days',
    'Northeast Region',
    b.id,
    'open',
    'high',
    true
FROM businesses b
WHERE b.business_type = 'distributor'
  AND b.is_active = COALESCE(b.is_active, true)
  AND NOT EXISTS (SELECT 1 FROM rfqs LIMIT 1)
LIMIT 1;
