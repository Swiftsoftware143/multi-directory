-- Add directory_id and processed_at columns to city_requests
-- This scopes city requests to specific directories and allows marking them as processed

ALTER TABLE city_requests
    ADD COLUMN IF NOT EXISTS directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    ADD COLUMN IF NOT EXISTS processed_at TIMESTAMPTZ;

-- Index for directory-scoped queries
CREATE INDEX IF NOT EXISTS idx_city_requests_directory_id ON city_requests(directory_id);
