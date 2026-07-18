-- Add CoreSwift tag mapping columns to plan_tiers
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS coreswift_tag TEXT;
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS coreswift_pipeline_stage TEXT;
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS slot_duration_days INTEGER DEFAULT 30;
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS max_slots_per_city INTEGER;

-- Slot inventory per city per plan
CREATE TABLE IF NOT EXISTS city_plan_slots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    city_slug TEXT NOT NULL,
    plan_tier_id UUID NOT NULL REFERENCES plan_tiers(id) ON DELETE CASCADE,
    total_slots INTEGER NOT NULL DEFAULT 10,
    filled_slots INTEGER NOT NULL DEFAULT 0,
    UNIQUE(city_slug, plan_tier_id)
);

-- Slot bookings (actual reservations with date ranges)
CREATE TABLE IF NOT EXISTS plan_slot_bookings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    city_slug TEXT NOT NULL,
    plan_tier_id UUID NOT NULL REFERENCES plan_tiers(id) ON DELETE CASCADE,
    business_id UUID REFERENCES businesses(id) ON DELETE SET NULL,
    business_name TEXT NOT NULL,
    contact_email TEXT NOT NULL,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    slot_position INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    price_paid NUMERIC(10,2),
    currency TEXT DEFAULT 'USD',
    coreswift_contact_id UUID,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_slot_bookings_city_plan ON plan_slot_bookings(city_slug, plan_tier_id);
CREATE INDEX IF NOT EXISTS idx_slot_bookings_active ON plan_slot_bookings(city_slug, plan_tier_id, status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_slot_bookings_date ON plan_slot_bookings(start_date, end_date);
