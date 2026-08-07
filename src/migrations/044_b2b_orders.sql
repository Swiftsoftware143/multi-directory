-- B2B Orders table for supplier-buyer marketplace
CREATE TABLE IF NOT EXISTS b2b_orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    buyer_business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    supplier_business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    product_id UUID NOT NULL REFERENCES supplier_products(id) ON DELETE CASCADE,
    quantity INTEGER NOT NULL DEFAULT 1,
    unit_price NUMERIC(10,2),
    total_amount NUMERIC(10,2),
    status TEXT NOT NULL DEFAULT 'pending',
    buyer_notes TEXT,
    delivery_area TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    shipped_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_supplier ON b2b_orders(supplier_business_id, status);
CREATE INDEX IF NOT EXISTS idx_b2b_orders_buyer ON b2b_orders(buyer_business_id);
CREATE INDEX IF NOT EXISTS idx_b2b_orders_status ON b2b_orders(status);

-- Extend business_messages for B2B messaging
ALTER TABLE business_messages ADD COLUMN IF NOT EXISTS sender_business_id UUID REFERENCES businesses(id) ON DELETE SET NULL;
ALTER TABLE business_messages ADD COLUMN IF NOT EXISTS to_business_id UUID REFERENCES businesses(id) ON DELETE CASCADE;

CREATE INDEX IF NOT EXISTS idx_business_messages_to_business ON business_messages(to_business_id);
CREATE INDEX IF NOT EXISTS idx_business_messages_sender_business ON business_messages(sender_business_id);
