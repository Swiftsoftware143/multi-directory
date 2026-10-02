-- 108: materialise the industries taxonomy locally.
--
-- GET /api/v1/industries/available (src/handlers/industries.rs) selected from
-- `template_categories` on this app's OWN pool (multi_directory), where no such table ever
-- existed: the query 42P01'd on every call and the handler silently served a hardcoded list.
-- The name exists in the workflowswift database, but MD reading another app's DB is not
-- acceptable here (one-writer-per-app: MD must not depend on WorkflowSwift being installed,
-- and it would need a server-wide WS_DATABASE_URL to do it). The taxonomy is therefore
-- materialised in multi_directory and read locally; the app now propagates a query error
-- instead of masking it with a fallback.
--
-- Seeded with the canonical list the endpoint has served since launch (the former hardcoded
-- fallback), so the contract is unchanged but now real: a plain table an admin tool can edit.

CREATE TABLE IF NOT EXISTS template_categories (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        text NOT NULL UNIQUE,
    name        text NOT NULL,
    description text,
    icon        varchar(50) DEFAULT '📁',
    sort_order  integer NOT NULL DEFAULT 0,
    is_active   boolean NOT NULL DEFAULT true,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

INSERT INTO template_categories (slug, name, description, icon, sort_order) VALUES
    ('sales-lead-gen',            'Sales & Lead Generation',   'Lead capture, nurturing, and sales pipeline automation',        '💼', 0),
    ('service-businesses',        'Service Businesses',        'Estimate, schedule, invoice workflows',                        '🔧', 1),
    ('recruitment-staffing',      'Recruitment & Staffing',    'Resume screening, interview coordination, placements',          '👥', 2),
    ('marketing-agencies',        'Marketing Agencies',        'Content calendars, ad campaigns, reporting',                    '📣', 3),
    ('professional-services',     'Professional Services',     'Tax, legal, consulting workflows',                              '⚖️', 4),
    ('ecommerce-retail',          'Ecommerce & Retail',        'Order fulfillment, inventory, dropshipping',                    '🛒', 5),
    ('healthcare-wellness',       'Healthcare & Wellness',     'Patient intake, appointments, treatment planning',              '🏥', 6),
    ('construction-development',  'Construction & Development', 'Permit management, subcontractor bidding, development',        '🏗️', 7),
    ('grant-funding',             'Grant & Funding',           'Grant writing, research, submission tracking',                  '💰', 8),
    ('education-training',        'Education & Training',      'Course creation, enrollment, certificates',                     '📚', 9),
    ('publishing-media',          'Publishing & Media',        'Content approval, newsletters, editorial calendars',            '📰', 10),
    ('site-flipping',             'Site Flipping',             'Website flipping, marketplace listings, TinyBrander funnel',    '🔄', 11),
    ('government-contracting',    'Government Contracting',    'Opportunity discovery, bidding, contract management',          '🏛️', 12),
    ('content-creation',          'Content Creation',          'AI video, images, voiceover workflows',                         '🎬', 13),
    ('newsletter',                'Newsletter',                'Email newsletter creation and management',                      '📧', 14)
ON CONFLICT (slug) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_template_categories_active
    ON template_categories (sort_order) WHERE is_active;
