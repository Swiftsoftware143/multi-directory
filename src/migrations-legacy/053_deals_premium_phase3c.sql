-- Phase 3c: QR Codes, Template Variants, Social Share (Premium Gated)
-- Adds premium features columns to deals table

-- 1. Premium features gating
ALTER TABLE deals ADD COLUMN IF NOT EXISTS premium_features BOOLEAN NOT NULL DEFAULT false;

-- 2. Redemption type: 'code', 'qr', 'wallet', 'booking'
ALTER TABLE deals ADD COLUMN IF NOT EXISTS redemption_type VARCHAR(20) NOT NULL DEFAULT 'code';

-- 3. Booking URL for service/event bookings
ALTER TABLE deals ADD COLUMN IF NOT EXISTS booking_url TEXT;

-- 4. QR code display toggle
ALTER TABLE deals ADD COLUMN IF NOT EXISTS show_qr BOOLEAN NOT NULL DEFAULT false;

-- Update page_template constraint to include new Phase 3c templates
ALTER TABLE deals DROP CONSTRAINT IF EXISTS deals_page_template_check;
ALTER TABLE deals ADD CONSTRAINT deals_page_template_check
    CHECK (page_template = ANY (ARRAY[
        'classic'::text, 'modern'::text, 'bold'::text, 'minimal'::text,
        'service'::text, 'ecommerce'::text, 'event'::text
    ]));

-- Add check constraint for redemption_type
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'deals_redemption_type_check'
    ) THEN
        ALTER TABLE deals ADD CONSTRAINT deals_redemption_type_check
            CHECK (redemption_type IN ('code', 'qr', 'wallet', 'booking'));
    END IF;
END $$;
