CREATE TABLE IF NOT EXISTS call_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    caller_number TEXT,
    called_number TEXT,
    direction TEXT DEFAULT 'inbound',
    duration_seconds INTEGER DEFAULT 0,
    call_status TEXT DEFAULT 'missed',
    recording_url TEXT,
    transcription TEXT,
    business_id UUID,
    directory_id UUID REFERENCES directories(id),
    lead_name TEXT,
    lead_email TEXT,
    lead_notes TEXT,
    lead_status TEXT DEFAULT 'new',
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS twilio_numbers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    phone_number TEXT UNIQUE NOT NULL,
    friendly_name TEXT,
    sid TEXT,
    provider TEXT DEFAULT 'telnyx',
    directory_id UUID REFERENCES directories(id),
    business_id UUID,
    forwarding_number TEXT,
    webhook_url TEXT,
    call_logging BOOLEAN DEFAULT true,
    monthly_cost DECIMAL(8,2),
    status TEXT DEFAULT 'active',
    created_at TIMESTAMPTZ DEFAULT NOW()
);
