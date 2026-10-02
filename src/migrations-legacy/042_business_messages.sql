-- Business messages/contact form submissions
CREATE TABLE IF NOT EXISTS business_messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    sender_name TEXT,
    sender_email TEXT,
    subject TEXT,
    message TEXT NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_business_messages_business_id ON business_messages(business_id);
CREATE INDEX IF NOT EXISTS idx_business_messages_is_read ON business_messages(is_read);
