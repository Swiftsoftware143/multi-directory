-- Content Scheduling Queue for blog posts and trap door pages
-- Allows admins to schedule content weeks/months in advance, preview, edit, or cancel

CREATE TABLE IF NOT EXISTS content_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue_type TEXT NOT NULL CHECK (queue_type IN ('trap_door', 'blog')),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    keyword TEXT NOT NULL,
    template_id UUID,
    merge_fields JSONB,
    scheduled_for TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'generating', 'completed', 'failed', 'cancelled')),
    retry_count INTEGER DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_content_queue_status ON content_queue(status);
CREATE INDEX IF NOT EXISTS idx_content_queue_scheduled ON content_queue(scheduled_for);
