-- Migration 045: Order Fulfillment & Tracking
-- Adds tracking/delivery fields to b2b_orders and supplier analytics view

-- Add tracking/delivery fields to b2b_orders
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS tracking_number TEXT;
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS carrier TEXT;
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS estimated_delivery DATE;
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS actual_delivery_date DATE;
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS buyer_rating INTEGER CHECK (buyer_rating >= 1 AND buyer_rating <= 5);
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS buyer_review TEXT;
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS supplier_rating INTEGER CHECK (supplier_rating >= 1 AND supplier_rating <= 5);
ALTER TABLE b2b_orders ADD COLUMN IF NOT EXISTS supplier_review TEXT;

-- Supplier analytics view: aggregates per-supplier order stats
CREATE OR REPLACE VIEW supplier_order_stats AS
SELECT
    supplier_business_id,
    COUNT(*) as total_orders,
    COUNT(*) FILTER (WHERE status = 'pending') as pending_orders,
    COUNT(*) FILTER (WHERE status = 'confirmed') as confirmed_orders,
    COUNT(*) FILTER (WHERE status = 'shipped') as shipped_orders,
    COUNT(*) FILTER (WHERE status = 'delivered') as delivered_orders,
    COUNT(*) FILTER (WHERE status = 'cancelled') as cancelled_orders,
    COALESCE(SUM(total_amount) FILTER (WHERE status != 'cancelled'), 0) as total_revenue,
    COALESCE(AVG(buyer_rating), 0) as avg_rating
FROM b2b_orders
GROUP BY supplier_business_id;
