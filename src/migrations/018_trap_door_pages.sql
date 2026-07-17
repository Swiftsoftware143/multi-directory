-- Migration 018: Trap Door hyper-niche pages
-- Adds time/day dimensions + analytics to programmatic_pages
-- Creates trap_door_templates table for managing generation patterns

-- 1. Day-specific tags on programmatic_pages
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS day_tags TEXT[] DEFAULT '{}';

-- 2. Time-specific tags on programmatic_pages
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS time_tags TEXT[] DEFAULT '{}';

-- 3. Specific hour slot on programmatic_pages
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS hour_slot TEXT;

-- 4. Analytics tracking columns on programmatic_pages
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS impressions INTEGER DEFAULT 0;
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS clicks INTEGER DEFAULT 0;
ALTER TABLE programmatic_pages ADD COLUMN IF NOT EXISTS conversions INTEGER DEFAULT 0;

-- 5. Trap Door Templates table
CREATE TABLE IF NOT EXISTS trap_door_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    pattern TEXT NOT NULL,
    placeholders JSONB DEFAULT '[]',
    is_active BOOLEAN DEFAULT true,
    last_generated_at TIMESTAMPTZ,
    page_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trap_door_templates_directory ON trap_door_templates(directory_id);
CREATE INDEX IF NOT EXISTS idx_trap_door_templates_active ON trap_door_templates(is_active);
