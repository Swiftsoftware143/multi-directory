CREATE TABLE IF NOT EXISTS import_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type TEXT NOT NULL,
    filename TEXT,
    rows_total INTEGER DEFAULT 0,
    rows_success INTEGER DEFAULT 0,
    rows_failed INTEGER DEFAULT 0,
    errors JSONB DEFAULT '[]',
    directory_id UUID REFERENCES directories(id),
    status TEXT DEFAULT 'pending',
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS export_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    fields JSONB NOT NULL,
    directory_id UUID REFERENCES directories(id),
    delimiter TEXT DEFAULT ',',
    include_header BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
