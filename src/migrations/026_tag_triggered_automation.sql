-- Tag-Triggered Automation + Tracked Links
-- Phase: Marketing Automation

CREATE TABLE IF NOT EXISTS tag_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    tag_id UUID NOT NULL,
    trigger_type TEXT NOT NULL CHECK (trigger_type IN ('tag_applied', 'tag_removed', 'workflow_completed')),
    action_type TEXT NOT NULL CHECK (action_type IN ('send_email', 'send_sms', 'webhook', 'pipeline_move', 'scoring_update', 'add_tag', 'remove_tag', 'issue_voucher')),
    action_config JSONB NOT NULL DEFAULT '{}',
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_tag_rules_tenant ON tag_rules(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tag_rules_tag ON tag_rules(tag_id);

CREATE TABLE IF NOT EXISTS tracked_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    utm_source TEXT,
    utm_medium TEXT,
    utm_campaign TEXT,
    utm_content TEXT,
    short_code TEXT UNIQUE,
    is_active BOOLEAN DEFAULT true,
    total_clicks INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_tracked_links_tenant ON tracked_links(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tracked_links_short_code ON tracked_links(short_code);

CREATE TABLE IF NOT EXISTS link_clicks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    link_id UUID NOT NULL REFERENCES tracked_links(id) ON DELETE CASCADE,
    contact_id UUID,
    ip_address INET,
    user_agent TEXT,
    referer TEXT,
    country TEXT,
    city TEXT,
    device_type TEXT,
    clicked_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_link_clicks_link ON link_clicks(link_id);
CREATE INDEX IF NOT EXISTS idx_link_clicks_contact ON link_clicks(contact_id);
CREATE INDEX IF NOT EXISTS idx_link_clicks_clicked ON link_clicks(clicked_at);
