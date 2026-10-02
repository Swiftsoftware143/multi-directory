-- 071_schema_reconciliation_full: recreate missing tables + columns
-- Idempotent reconciliation for schema drift from the Docker migration runner

CREATE TABLE IF NOT EXISTS business_meta (

 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
 template VARCHAR(64) NOT NULL DEFAULT 'local-business',
 meta_data JSONB NOT NULL DEFAULT '{}',
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 UNIQUE(business_id, template)

);

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

CREATE TABLE IF NOT EXISTS sitemap_config (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    auto_generate BOOLEAN DEFAULT true,
    priority DECIMAL(2,1) DEFAULT 0.5,
    change_freq TEXT DEFAULT 'weekly',
    last_generated TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(directory_id)

);

CREATE TABLE IF NOT EXISTS analytics_events (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    entity_type TEXT,
    entity_id UUID,
    directory_id UUID REFERENCES directories(id) ON DELETE SET NULL,
    metadata JSONB,
    ip_address TEXT,
    user_agent TEXT,
    referrer TEXT,
    session_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS ad_zones (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    zone_key TEXT NOT NULL,
    width INTEGER DEFAULT 300,
    height INTEGER DEFAULT 250,
    price_monthly DECIMAL(10,2),
    directory_id UUID REFERENCES directories(id),
    status TEXT DEFAULT 'available',
    current_advertiser_id UUID,
    current_ad_url TEXT,
    current_ad_image TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS directory_surveys (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT false,
    title TEXT NOT NULL DEFAULT 'Help us personalize your experience',
    description TEXT DEFAULT '',
    -- JSON array of survey question objects: [{ "id": "q1", "type": "choice|multi|text", "label": "...", "options": [...], "tags": [...] }]
    questions JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Tags applied to visitor when they complete the survey
    completion_tags JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Which event triggers the survey (first_visit, after_n_visits, opt_in)
    trigger_event TEXT NOT NULL DEFAULT 'first_visit',
    -- Whether the survey is required before browsing
    required BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()

);

CREATE TABLE IF NOT EXISTS survey_responses (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    survey_id UUID NOT NULL REFERENCES directory_surveys(id) ON DELETE CASCADE,
    visitor_account_id UUID REFERENCES visitor_accounts(id) ON DELETE SET NULL,
    visitor_fingerprint TEXT,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    answers JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Tags that were applied as a result of this survey
    applied_tags TEXT[] NOT NULL DEFAULT '{}',
    completed_at TIMESTAMPTZ NOT NULL DEFAULT now()

);

CREATE TABLE IF NOT EXISTS directory_tiers (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    tier_slug TEXT NOT NULL,
    tier_name TEXT NOT NULL DEFAULT 'Free',
    is_active BOOLEAN DEFAULT true,
    started_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    stripe_subscription_id TEXT,
    stripe_customer_id TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(directory_id)

);

CREATE TABLE IF NOT EXISTS sponsored_listings (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    slot_position INTEGER NOT NULL DEFAULT 1,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    is_active BOOLEAN DEFAULT true,
    price_paid DECIMAL(10,2) DEFAULT 0,
    currency TEXT DEFAULT 'USD',
    stripe_payment_intent_id TEXT,
    featured BOOLEAN DEFAULT false,
    badge_text TEXT,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS landing_pages (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    directory_id UUID REFERENCES directories(id),
    hero_title TEXT,
    hero_subtitle TEXT,
    hero_cta_text TEXT,
    hero_cta_url TEXT,
    features JSONB DEFAULT '[]',
    testimonials JSONB DEFAULT '[]',
    faq JSONB DEFAULT '[]',
    seo_title TEXT,
    seo_description TEXT,
    published BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS public_themes (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    directory_id UUID REFERENCES directories(id),
    primary_color TEXT DEFAULT '#2563eb',
    secondary_color TEXT DEFAULT '#1e40af',
    header_style TEXT DEFAULT 'gradient',
    layout TEXT DEFAULT 'grid',
    show_search BOOLEAN DEFAULT true,
    show_categories BOOLEAN DEFAULT true,
    show_featured BOOLEAN DEFAULT true,
    items_per_page INTEGER DEFAULT 12,
    custom_css TEXT,
    custom_js TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS business_verifications (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID REFERENCES directories(id),
    method TEXT NOT NULL DEFAULT 'manual',
    status TEXT NOT NULL DEFAULT 'pending',
    verified_by UUID REFERENCES users(id),
    verified_at TIMESTAMPTZ,
    verification_doc_url TEXT,
    notes TEXT,
    expires_at TIMESTAMPTZ,
    verified_data JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS data_enrichment_logs (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID REFERENCES businesses(id) ON DELETE SET NULL,
    directory_id UUID REFERENCES directories(id),
    source TEXT NOT NULL,
    enrichment_type TEXT NOT NULL,
    data_before JSONB,
    data_after JSONB,
    confidence DOUBLE PRECISION DEFAULT 1.0,
    status TEXT NOT NULL DEFAULT 'completed',
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS webhooks (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id),
    tenant_id UUID REFERENCES tenants(id),
    directory_id UUID REFERENCES directories(id),
    url TEXT NOT NULL,
    events TEXT[] NOT NULL DEFAULT '{}',
    secret TEXT,
    is_active BOOLEAN DEFAULT true,
    retry_count INTEGER DEFAULT 3,
    timeout_seconds INTEGER DEFAULT 10,
    last_triggered_at TIMESTAMPTZ,
    last_success_at TIMESTAMPTZ,
    last_failure_at TIMESTAMPTZ,
    failure_count INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS webhook_deliveries (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_id UUID NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER DEFAULT 0,
    max_attempts INTEGER DEFAULT 3,
    response_status_code INTEGER,
    response_body TEXT,
    error_message TEXT,
    next_retry_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS blog_templates (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    description TEXT,
    category TEXT NOT NULL DEFAULT 'seo',  -- 'seo', 'geo', 'aeo', 'listicle', 'howto', 'faq', 'guide', 'news'
    content_template TEXT NOT NULL,
    merge_fields JSONB DEFAULT '[]'::jsonb,  -- configurable fields like [{"key":"city","label":"City"}, ...]
    is_global BOOLEAN DEFAULT true,
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,  -- NULL = global, set = per-directory
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS newsletter_queue (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    intro_text TEXT,
    include_blog BOOLEAN DEFAULT true,
    include_deals BOOLEAN DEFAULT true,
    manual_sections JSONB DEFAULT '[]'::jsonb,  -- manual ad spots / custom content
    scheduled_at TIMESTAMPTZ,
    sent_at TIMESTAMPTZ,
    status TEXT DEFAULT 'draft',  -- 'draft', 'scheduled', 'sent'
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS homepage_sections (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    network_id UUID REFERENCES networks(id) ON DELETE CASCADE,
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    section_type VARCHAR(50) NOT NULL,
    sort_order INT DEFAULT 0,
    title VARCHAR(255),
    subtitle TEXT,
    content TEXT,
    cta_text VARCHAR(100),
    cta_url VARCHAR(500),
    image_url TEXT,
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT homepage_owner_check CHECK (
        (network_id IS NOT NULL AND directory_id IS NULL) OR
        (network_id IS NULL AND directory_id IS NOT NULL)
    )

);

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

CREATE TABLE IF NOT EXISTS checkout_sessions (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_type TEXT NOT NULL,
    provider_session_id TEXT,
    -- What's being purchased
    purchasable_type TEXT NOT NULL CHECK (purchasable_type IN ('plan_subscription', 'sponsored_listing', 'ad_zone', 'credits')),
    purchasable_id UUID,
    -- Business/directory context
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID REFERENCES directories(id) ON DELETE SET NULL,
    -- Pricing
    amount NUMERIC(10,2) NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    -- Status tracking
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed', 'failed', 'expired', 'refunded')),
    -- Webhook verification
    webhook_received_at TIMESTAMPTZ,
    webhook_event_id TEXT,
    -- Metadata
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()

);

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

CREATE TABLE IF NOT EXISTS service_prices (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    network_id UUID REFERENCES networks(id) ON DELETE CASCADE,
    service_key VARCHAR(100) NOT NULL,
    price_monthly NUMERIC(10,2),
    price_yearly NUMERIC(10,2),
    price_one_time NUMERIC(10,2),
    currency VARCHAR(3) DEFAULT 'USD',
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    CONSTRAINT unique_service_per_scope UNIQUE (directory_id, network_id, service_key),
    CONSTRAINT check_scope CHECK (
        (directory_id IS NOT NULL AND network_id IS NULL) OR
        (directory_id IS NULL AND network_id IS NOT NULL) OR
        (directory_id IS NULL AND network_id IS NULL)
    )

);

CREATE TABLE IF NOT EXISTS price_bundles (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID REFERENCES directories(id) ON DELETE CASCADE,
    network_id UUID REFERENCES networks(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) UNIQUE NOT NULL,
    description TEXT,
    price_monthly NUMERIC(10,2),
    price_yearly NUMERIC(10,2),
    is_active BOOLEAN DEFAULT true,
    sort_order INTEGER DEFAULT 0,
    is_featured BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS bundle_services (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id UUID NOT NULL REFERENCES price_bundles(id) ON DELETE CASCADE,
    service_key VARCHAR(100) NOT NULL,
    UNIQUE(bundle_id, service_key)

);

CREATE TABLE IF NOT EXISTS service_bookings (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    visitor_account_id UUID NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    service_name TEXT,
    description TEXT,
    preferred_date TIMESTAMPTZ,
    preferred_time TEXT,
    contact_phone TEXT,
    contact_email TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS account_links (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL,
    visitor_account_id UUID REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(email, visitor_account_id),
    UNIQUE(email, user_id)

);

CREATE TABLE IF NOT EXISTS business_services (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    price NUMERIC(10,2),
    currency TEXT NOT NULL DEFAULT 'USD',
    duration_minutes INTEGER,
    category TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS sponsors (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','active','suspended','inactive')),
    commission_rate DECIMAL(5,2) DEFAULT 0,
    notes TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(directory_id, business_id)

);

CREATE TABLE IF NOT EXISTS ad_creatives (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sponsor_id UUID NOT NULL REFERENCES sponsors(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    image_url TEXT NOT NULL,
    target_url TEXT,
    width INTEGER NOT NULL CHECK (width > 0),
    height INTEGER NOT NULL CHECK (height > 0),
    mime_type TEXT DEFAULT 'image/png',
    file_size_bytes INTEGER,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected','archived')),
    rejection_reason TEXT,
    is_active BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS ad_schedules (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    ad_zone_id UUID NOT NULL REFERENCES ad_zones(id) ON DELETE CASCADE,
    sponsor_id UUID NOT NULL REFERENCES sponsors(id) ON DELETE CASCADE,
    creative_id UUID NOT NULL REFERENCES ad_creatives(id) ON DELETE CASCADE,
    start_date TIMESTAMPTZ NOT NULL,
    end_date TIMESTAMPTZ NOT NULL CHECK (end_date > start_date),
    price_monthly DECIMAL(10,2) NOT NULL DEFAULT 0.00,
    total_price DECIMAL(10,2) NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','active','completed','cancelled')),
    auto_renew BOOLEAN DEFAULT false,
    created_by UUID, -- user who created the schedule
    approved_at TIMESTAMPTZ,
    approved_by UUID,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS ad_earnings (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES ad_schedules(id) ON DELETE CASCADE,
    sponsor_id UUID NOT NULL REFERENCES sponsors(id) ON DELETE CASCADE,
    ad_zone_id UUID NOT NULL REFERENCES ad_zones(id) ON DELETE CASCADE,
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    amount DECIMAL(10,2) NOT NULL,
    period_start DATE NOT NULL,
    period_end DATE NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','paid','overdue','cancelled')),
    paid_at TIMESTAMPTZ,
    notes TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS approval_queue (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL CHECK (item_type IN ('sponsor','ad_creative','ad_schedule','featured_listing','subscription')),
    item_id UUID NOT NULL,
    submitted_by UUID,
    submitted_at TIMESTAMPTZ DEFAULT NOW(),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected')),
    reviewed_by UUID,
    reviewed_at TIMESTAMPTZ,
    notes TEXT,
    UNIQUE(item_type, item_id)

);

CREATE TABLE IF NOT EXISTS directory_notifications (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    message TEXT NOT NULL,
    link_text TEXT,
    link_url TEXT,
    notification_type TEXT NOT NULL DEFAULT 'info',
    is_active BOOLEAN NOT NULL DEFAULT true,
    starts_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()

);

CREATE TABLE IF NOT EXISTS supplier_products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    category TEXT,
    price NUMERIC(12,2),
    unit TEXT,
    min_order INTEGER DEFAULT 1,
    currency TEXT DEFAULT 'USD',
    delivery_areas TEXT[] DEFAULT '{}',
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_supplier_products_business ON supplier_products(business_id);
CREATE INDEX IF NOT EXISTS idx_supplier_products_category ON supplier_products(category);

CREATE TABLE IF NOT EXISTS b2b_orders (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    buyer_business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    supplier_business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    product_id UUID NOT NULL REFERENCES supplier_products(id) ON DELETE CASCADE,
    quantity INTEGER NOT NULL DEFAULT 1,
    unit_price NUMERIC(10,2),
    total_amount NUMERIC(10,2),
    status TEXT NOT NULL DEFAULT 'pending',
    buyer_notes TEXT,
    delivery_area TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    shipped_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ

);

CREATE TABLE IF NOT EXISTS b2b_notifications (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    related_order_id UUID REFERENCES b2b_orders(id) ON DELETE SET NULL,
    related_message_id UUID REFERENCES business_messages(id) ON DELETE SET NULL,
    is_read BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()

);

CREATE TABLE IF NOT EXISTS event_providers (

    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('eventbrite', 'meetup', 'ics_feed', 'n8n_webhook')),
    api_key TEXT,
    config JSONB NOT NULL DEFAULT '{}',
    is_active BOOLEAN NOT NULL DEFAULT true,
    last_sync_at TIMESTAMPTZ,
    last_sync_status TEXT,
    last_error TEXT,
    events_synced INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()

);

ALTER TABLE directory_categories ADD COLUMN IF NOT EXISTS group_name TEXT;
CREATE TABLE IF NOT EXISTS business_categories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES directory_categories(id) ON DELETE CASCADE,
    is_primary BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(business_id, category_id)
);
CREATE INDEX IF NOT EXISTS idx_business_categories_business ON business_categories(business_id);
CREATE INDEX IF NOT EXISTS idx_business_categories_category ON business_categories(category_id);

