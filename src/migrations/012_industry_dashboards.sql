-- Industry Dashboards for Multi-Directory Portal
-- Links each user/tenant to industry dashboard configurations
-- Industry slugs align with template_categories in workflowswift DB

-- User industry dashboards: tracks which industries a user has activated
CREATE TABLE IF NOT EXISTS user_industry_dashboards (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    industry_slug VARCHAR(100) NOT NULL,
    dashboard_name VARCHAR(255) NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, industry_slug)
);

CREATE INDEX IF NOT EXISTS idx_user_industry_dashboards_user ON user_industry_dashboards(user_id);
CREATE INDEX IF NOT EXISTS idx_user_industry_dashboards_tenant ON user_industry_dashboards(tenant_id);
CREATE INDEX IF NOT EXISTS idx_user_industry_dashboards_industry ON user_industry_dashboards(industry_slug);

-- Industry plan limits: how many industries each plan tier allows
-- (already handled via feature_limits table in workflowswift, but we add a column here for convenience)
ALTER TABLE plan_tiers ADD COLUMN IF NOT EXISTS max_industries INTEGER DEFAULT 1;

-- Tenant default industry (for new users on this tenant)
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS industry_slug VARCHAR(100) DEFAULT 'site-flipping';

INSERT INTO _migrations (filename) VALUES ('012_industry_dashboards.sql')
ON CONFLICT (filename) DO NOTHING;
