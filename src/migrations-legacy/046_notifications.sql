CREATE TABLE IF NOT EXISTS b2b_notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    related_order_id UUID REFERENCES b2b_orders(id) ON DELETE SET NULL,
    related_message_id UUID REFERENCES business_messages(id) ON DELETE SET NULL,
    is_read BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_b2b_notifications_business ON b2b_notifications(business_id, is_read);
CREATE INDEX IF NOT EXISTS idx_b2b_notifications_created ON b2b_notifications(created_at DESC);
