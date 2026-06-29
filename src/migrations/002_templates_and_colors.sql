-- Add template and color_scheme columns to directories
ALTER TABLE directories ADD COLUMN IF NOT EXISTS template VARCHAR(64) DEFAULT 'local-business';
ALTER TABLE directories ADD COLUMN IF NOT EXISTS color_scheme JSONB DEFAULT '{" primary\:\#2563eb\,\secondary\:\#64748b\,\accent\:\#f59e0b\,\background\:\#ffffff\,\text\:\#1e293b\,\heading\:\#0f172a\,\muted\:\#94a3b8\,\border\:\#e2e8f0\}';

-- Create business_meta table for template-specific fields
CREATE TABLE IF NOT EXISTS business_meta (
 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
 template VARCHAR(64) NOT NULL DEFAULT 'local-business',
 meta_data JSONB NOT NULL DEFAULT '{}',
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 UNIQUE(business_id, template)
);

CREATE INDEX IF NOT EXISTS idx_business_meta_business ON business_meta(business_id);
CREATE INDEX IF NOT EXISTS idx_business_meta_template ON business_meta(template);

INSERT INTO _migrations (filename) VALUES ('002_templates_and_colors.sql')
ON CONFLICT (filename) DO NOTHING;
