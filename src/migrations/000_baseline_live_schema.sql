-- 000_baseline_live_schema.sql
-- multi-directory baseline: the shape this database ACTUALLY has.
--
-- WHY THIS FILE EXISTS (kanban t_979f881e, measured 2026-10-02; evidence
-- /opt/swift/audits/t_cb86753e/40-fresh-install-finding.txt)
--   `src/migrations/` was not this app's install path. The live database was created out of band
--   and the 124 delta files were then layered on top of it (all 124 are recorded in `_migrations`
--   with had_errors=false on live), so the chain had never been asked to build a schema from
--   scratch. On an EMPTY database only 34 of the 124 files applied and 90 FAILED: `001_initial.sql`
--   creates just tenants/users/password_resets/_migrations, then `002_templates_and_colors.sql`
--   ALTERs `directories` and creates `business_meta REFERENCES businesses(id)` — neither table
--   exists yet (the core baseline is not in the chain at all), so the chain collapses and
--   `businesses` is never created. The API still answered /api/v1/health 200 on that hollow
--   install, because the migrator is deliberately non-fatal (src/db.rs). A buyer with no agent
--   would have received a running but unusable directory.
--
-- WHAT THIS FILE IS
--   The live catalog - every table, column, constraint, index, sequence, view, function, trigger
--   and column comment - generated FROM the live database by scripts/gen-baseline-from-live.py.
--   It is the SOURCE OF TRUTH for a fresh install or a restore. Every schema change from here on
--   ships as a NEW higher-numbered file beside it (see src/migrations/README.md); never edit this
--   one. The superseded delta chain is preserved in src/migrations-legacy/.
--
-- ON THE LIVE DATABASE EVERY STATEMENT IS A NO-OP
--   CREATE TABLE/SEQUENCE/INDEX IF NOT EXISTS, CREATE OR REPLACE VIEW/FUNCTION, and a constraint or
--   trigger is created only when the catalog does not already hold it. Applying this file to the
--   database it was generated from creates nothing, changes nothing and validates nothing
--   (whole-catalog fingerprint before/after: /opt/swift/audits/t_979f881e/).
--
-- NOT INCLUDED ON PURPOSE: `_migrations`, the runner's ledger (created at boot by
-- src/db.rs::run_migrations). It is bookkeeping, not application schema.


CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;

COMMENT ON EXTENSION pgcrypto IS 'cryptographic functions';

CREATE OR REPLACE FUNCTION public.auto_record_directory_event() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
    DECLARE
        v_event_type text;
        v_entity_type text;
        v_entity_id uuid;
        v_directory_id uuid;
        v_data jsonb;
    BEGIN
        -- Determine event type based on operation
        IF TG_OP = 'INSERT' THEN
            v_event_type := TG_TABLE_NAME || '.created';
            v_entity_id := NEW.id;
        ELSIF TG_OP = 'UPDATE' THEN
            v_event_type := TG_TABLE_NAME || '.updated';
            v_entity_id := NEW.id;
        ELSIF TG_OP = 'DELETE' THEN
            v_event_type := TG_TABLE_NAME || '.deleted';
            v_entity_id := OLD.id;
        END IF;
        
        -- Determine entity type and directory ID
        IF TG_TABLE_NAME = 'directories' THEN
            v_entity_type := 'directory';
            v_directory_id := v_entity_id;
            v_data := jsonb_build_object('slug', NEW.slug, 'name', NEW.name);
        ELSIF TG_TABLE_NAME = 'businesses' THEN
            v_entity_type := 'business';
            v_directory_id := COALESCE(NEW.directory_id, OLD.directory_id);
            v_data := jsonb_build_object('name', COALESCE(NEW.name, OLD.name));
        ELSIF TG_TABLE_NAME = 'directory_categories' THEN
            v_entity_type := 'category';
            v_directory_id := COALESCE(NEW.directory_id, OLD.directory_id);
            v_data := jsonb_build_object('name', COALESCE(NEW.name, OLD.name));
        ELSE
            RETURN COALESCE(NEW, OLD);
        END IF;
        
        -- Only record events for our zaarhub cities
        IF v_directory_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM directories 
            WHERE id = v_directory_id 
            AND slug IN ('apopka','boca-raton','hollywood','lake-nona',
                         'palm-bay','palm-coast','pompano-beach','st-cloud',
                         'st-petersburg','winter-garden')
        ) THEN
            INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, data)
            VALUES (v_event_type, v_entity_type, v_entity_id, v_directory_id, v_data);
        END IF;
        
        RETURN COALESCE(NEW, OLD);
    END;
    $$;

CREATE OR REPLACE FUNCTION public.businesses_search_update() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    cat_name text;
BEGIN
    SELECT COALESCE(name, '') INTO cat_name FROM directory_categories WHERE id = NEW.category_id;
    NEW.search_vector := to_tsvector('english',
        COALESCE(NEW.name, '') || ' ' ||
        COALESCE(NEW.description, '') || ' ' ||
        COALESCE(cat_name, '') || ' ' ||
        COALESCE(NEW.city, '') || ' ' ||
        COALESCE(NEW.state, '')
    );
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.decrypt_provider_key(encrypted_data bytea) RETURNS text
    LANGUAGE plpgsql
    AS $$
DECLARE
    enc_key TEXT;
BEGIN
    SELECT encryption_key INTO enc_key FROM app_encryption_config WHERE active = true LIMIT 1;
    IF enc_key IS NULL THEN
        RAISE EXCEPTION 'No active encryption config found';
    END IF;
    RETURN pgp_sym_decrypt(encrypted_data, enc_key);
END;
$$;

CREATE OR REPLACE FUNCTION public.encrypt_provider_key() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    enc_key TEXT;
BEGIN
    SELECT encryption_key INTO enc_key FROM app_encryption_config WHERE active = true LIMIT 1;
    IF enc_key IS NULL THEN
        RAISE EXCEPTION 'No active encryption config found';
    END IF;
    
    NEW.api_key_encrypted := pgp_sym_encrypt(NEW.api_key, enc_key);
    IF NEW.base_url IS NOT NULL AND NEW.base_url != '' THEN
        NEW.base_url_encrypted := pgp_sym_encrypt(NEW.base_url, enc_key);
    END IF;
    
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.notify_zaarhub_change() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
        BEGIN
            PERFORM pg_notify('zaarhub_site_changes', NOW()::text);
            RETURN NEW;
        END;
        $$;

CREATE OR REPLACE FUNCTION public.update_business_services_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.update_referrals_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION public.update_service_bookings_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE TABLE IF NOT EXISTS public._city_tags (
    directory_id uuid NOT NULL,
    tag_name text NOT NULL,
    tag_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.account_links (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    email text NOT NULL,
    visitor_account_id uuid,
    user_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.ad_creatives (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    sponsor_id uuid NOT NULL,
    name text NOT NULL,
    image_url text NOT NULL,
    target_url text,
    width integer NOT NULL,
    height integer NOT NULL,
    mime_type text DEFAULT 'image/png'::text,
    file_size_bytes integer,
    status text DEFAULT 'pending'::text NOT NULL,
    rejection_reason text,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    CONSTRAINT ad_creatives_height_check CHECK ((height > 0)),
    CONSTRAINT ad_creatives_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'approved'::text, 'rejected'::text, 'archived'::text]))),
    CONSTRAINT ad_creatives_width_check CHECK ((width > 0))
);

CREATE TABLE IF NOT EXISTS public.ad_earnings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    schedule_id uuid NOT NULL,
    sponsor_id uuid NOT NULL,
    ad_zone_id uuid NOT NULL,
    directory_id uuid NOT NULL,
    amount numeric(10,2) NOT NULL,
    period_start date NOT NULL,
    period_end date NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    paid_at timestamp with time zone,
    notes text,
    created_at timestamp with time zone DEFAULT now(),
    CONSTRAINT ad_earnings_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'paid'::text, 'overdue'::text, 'cancelled'::text])))
);

CREATE TABLE IF NOT EXISTS public.ad_schedules (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    ad_zone_id uuid NOT NULL,
    sponsor_id uuid NOT NULL,
    creative_id uuid NOT NULL,
    start_date timestamp with time zone NOT NULL,
    end_date timestamp with time zone NOT NULL,
    price_monthly numeric(10,2) DEFAULT 0.00 NOT NULL,
    total_price numeric(10,2) DEFAULT 0 NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    auto_renew boolean DEFAULT false,
    created_by uuid,
    approved_at timestamp with time zone,
    approved_by uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    CONSTRAINT ad_schedules_check CHECK ((end_date > start_date)),
    CONSTRAINT ad_schedules_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'active'::text, 'completed'::text, 'cancelled'::text])))
);

CREATE TABLE IF NOT EXISTS public.ad_zones (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    zone_key text NOT NULL,
    width integer DEFAULT 300,
    height integer DEFAULT 250,
    price_monthly numeric(10,2),
    directory_id uuid,
    status text DEFAULT 'available'::text,
    current_advertiser_id uuid,
    current_ad_url text,
    current_ad_image text,
    created_at timestamp with time zone DEFAULT now(),
    external_payment_ref text
);

CREATE TABLE IF NOT EXISTS public.analytics_events (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    event_type text NOT NULL,
    entity_type text,
    entity_id uuid,
    directory_id uuid,
    metadata jsonb,
    ip_address text,
    user_agent text,
    referrer text,
    session_id text,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.api_key_usage (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    api_key_id uuid NOT NULL,
    endpoint text NOT NULL,
    method text NOT NULL,
    status_code integer,
    ip_address text,
    response_time_ms integer,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.api_keys (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_id uuid NOT NULL,
    tenant_id uuid,
    name text NOT NULL,
    key_hash text NOT NULL,
    key_prefix text NOT NULL,
    scopes text[] DEFAULT '{}'::text[],
    rate_limit_per_minute integer DEFAULT 60,
    rate_limit_per_hour integer DEFAULT 1000,
    is_active boolean DEFAULT true,
    last_used_at timestamp with time zone,
    expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.app_encryption_config (
    id integer NOT NULL,
    active boolean DEFAULT true NOT NULL,
    encryption_key text DEFAULT (gen_random_uuid())::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    rotated_at timestamp with time zone
);

CREATE SEQUENCE IF NOT EXISTS public.app_encryption_config_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.app_encryption_config_id_seq OWNED BY public.app_encryption_config.id;

CREATE TABLE IF NOT EXISTS public.approval_queue (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    item_type text NOT NULL,
    item_id uuid NOT NULL,
    submitted_by uuid,
    submitted_at timestamp with time zone DEFAULT now(),
    status text DEFAULT 'pending'::text NOT NULL,
    reviewed_by uuid,
    reviewed_at timestamp with time zone,
    notes text,
    CONSTRAINT approval_queue_item_type_check CHECK ((item_type = ANY (ARRAY['sponsor'::text, 'ad_creative'::text, 'ad_schedule'::text, 'featured_listing'::text, 'subscription'::text]))),
    CONSTRAINT approval_queue_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'approved'::text, 'rejected'::text])))
);

CREATE TABLE IF NOT EXISTS public.author_profiles (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    user_id uuid,
    name text NOT NULL,
    slug text NOT NULL,
    bio text,
    avatar_url text,
    twitter_url text,
    linkedin_url text,
    website_url text,
    role text DEFAULT 'author'::text,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    CONSTRAINT author_profiles_role_check CHECK ((role = ANY (ARRAY['directory_admin'::text, 'editor'::text, 'author'::text, 'guest_contributor'::text])))
);

CREATE TABLE IF NOT EXISTS public.available_providers (
    key character varying(64) NOT NULL,
    name character varying(128) NOT NULL,
    description text,
    requires_base_url boolean DEFAULT false,
    requires_metadata jsonb DEFAULT '[]'::jsonb,
    icon character varying(32),
    field_label text,
    field_help text,
    browser_only boolean DEFAULT false NOT NULL
);

CREATE TABLE IF NOT EXISTS public.b2b_notifications (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    type text NOT NULL,
    title text NOT NULL,
    body text,
    related_order_id uuid,
    related_message_id uuid,
    is_read boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.b2b_orders (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    buyer_business_id uuid NOT NULL,
    supplier_business_id uuid NOT NULL,
    product_id uuid NOT NULL,
    quantity integer DEFAULT 1 NOT NULL,
    unit_price numeric(10,2),
    total_amount numeric(10,2),
    status text DEFAULT 'pending'::text NOT NULL,
    buyer_notes text,
    delivery_area text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    confirmed_at timestamp with time zone,
    shipped_at timestamp with time zone,
    delivered_at timestamp with time zone,
    tracking_number text,
    carrier text,
    estimated_delivery date,
    actual_delivery_date date,
    buyer_rating integer,
    buyer_review text,
    supplier_rating integer,
    supplier_review text,
    CONSTRAINT b2b_orders_buyer_rating_check CHECK (((buyer_rating >= 1) AND (buyer_rating <= 5))),
    CONSTRAINT b2b_orders_supplier_rating_check CHECK (((supplier_rating >= 1) AND (supplier_rating <= 5)))
);

CREATE TABLE IF NOT EXISTS public.blog_media (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    blog_post_id uuid,
    media_type character varying(20) DEFAULT 'image'::character varying NOT NULL,
    url text NOT NULL,
    alt_text text,
    source character varying(50) DEFAULT 'ai_generated'::character varying,
    "position" integer DEFAULT 0,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.blog_posts (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    slug text,
    excerpt text,
    content text NOT NULL,
    directory_id uuid NOT NULL,
    published boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    focus_keyword text,
    meta_title text,
    meta_description text,
    canonical_url text,
    robots_meta text DEFAULT 'index'::text,
    featured_image_url text,
    featured_image_alt text,
    schema_type text DEFAULT 'Article'::text,
    author_id uuid,
    service_id uuid,
    location_id uuid,
    scheduled_at timestamp with time zone,
    template_id uuid,
    template_data jsonb DEFAULT '{}'::jsonb,
    is_master boolean DEFAULT false,
    master_post_id uuid,
    blog_category text DEFAULT 'general'::text,
    tags text[] DEFAULT '{}'::text[],
    feature_image text,
    feature_video text,
    media_json jsonb DEFAULT '[]'::jsonb,
    post_type text DEFAULT 'blog'::text,
    author_name text,
    status text DEFAULT 'published'::text,
    mentioned_business_ids uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    last_refreshed timestamp with time zone,
    page_views integer DEFAULT 0 NOT NULL,
    traffic_trend text,
    decay_flag boolean DEFAULT false NOT NULL,
    refresh_priority text,
    answer_block text,
    aeo_score integer DEFAULT 0 NOT NULL,
    internal_links jsonb DEFAULT '[]'::jsonb NOT NULL,
    schema_json jsonb,
    CONSTRAINT blog_posts_post_type_check CHECK ((post_type = ANY (ARRAY['blog'::text, 'community'::text, 'article'::text]))),
    CONSTRAINT blog_posts_schema_type_check CHECK ((schema_type = ANY (ARRAY['Article'::text, 'HowTo'::text, 'FAQPage'::text]))),
    CONSTRAINT blog_posts_status_check CHECK ((status = ANY (ARRAY['draft'::text, 'pending_review'::text, 'published'::text, 'archived'::text])))
);

CREATE TABLE IF NOT EXISTS public.blog_qa_keywords (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    question text NOT NULL,
    keyword text NOT NULL,
    intent text DEFAULT 'question'::text,
    source text DEFAULT 'manual'::text,
    frequency integer DEFAULT 0,
    target_category text,
    status text DEFAULT 'unused'::text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.blog_qa_posts (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    blog_post_id uuid,
    question text NOT NULL,
    keyword text NOT NULL,
    ai_model text,
    template_id text,
    status text DEFAULT 'draft'::text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.blog_template_directories (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    template_id uuid NOT NULL,
    directory_id uuid NOT NULL
);

CREATE TABLE IF NOT EXISTS public.blog_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    description text,
    category text DEFAULT 'seo'::text NOT NULL,
    content_template text NOT NULL,
    merge_fields jsonb DEFAULT '[]'::jsonb,
    is_global boolean DEFAULT true,
    directory_id uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    template_type character varying(50) DEFAULT 'article'::character varying,
    llm_provider character varying(50) DEFAULT 'deepseek'::character varying,
    llm_model character varying(100) DEFAULT 'deepseek-chat'::character varying,
    image_provider character varying(50) DEFAULT 'none'::character varying,
    image_model character varying(100) DEFAULT 'none'::character varying,
    word_count integer DEFAULT 1000,
    is_admin boolean DEFAULT false,
    status character varying(20) DEFAULT 'active'::character varying
);

CREATE TABLE IF NOT EXISTS public.bundle_services (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    bundle_id uuid NOT NULL,
    service_key character varying(100) NOT NULL
);

CREATE TABLE IF NOT EXISTS public.business_articles (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    business_id uuid,
    title text NOT NULL,
    slug text NOT NULL,
    keyword text NOT NULL,
    meta_description text,
    content text,
    status text DEFAULT 'draft'::text,
    impressions integer DEFAULT 0,
    clicks integer DEFAULT 0,
    is_owner_article boolean DEFAULT false,
    subscription_active boolean DEFAULT false,
    subscription_expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.business_categories (
    business_id uuid NOT NULL,
    category_id uuid NOT NULL,
    is_primary boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.business_listings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    city_page_id uuid NOT NULL,
    business_name character varying(255) NOT NULL,
    category character varying(255),
    subcategory character varying(255),
    description text,
    address character varying(255),
    phone character varying(50),
    website character varying(500),
    logo_url text,
    cover_image_url text,
    rating double precision DEFAULT 0,
    review_count integer DEFAULT 0 NOT NULL,
    is_featured boolean DEFAULT false NOT NULL,
    is_claimed boolean DEFAULT false NOT NULL,
    deal_text text,
    deal_url text,
    coordinates_lat double precision,
    coordinates_lng double precision,
    display_order integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    is_editors_pick boolean DEFAULT false NOT NULL,
    editors_pick_note text
);

CREATE TABLE IF NOT EXISTS public.business_messages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    sender_name text,
    sender_email text,
    subject text,
    message text NOT NULL,
    is_read boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    sender_business_id uuid,
    to_business_id uuid
);

CREATE TABLE IF NOT EXISTS public.business_meta (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    template character varying(64) DEFAULT 'local-business'::character varying NOT NULL,
    meta_data jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.business_point_ledger (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    business_id uuid NOT NULL,
    business_name text,
    month_key character varying(7) NOT NULL,
    points_issued_this_month bigint DEFAULT 0 NOT NULL,
    points_redeemed_this_month bigint DEFAULT 0 NOT NULL,
    total_billed_this_month numeric(14,2) DEFAULT 0 NOT NULL,
    total_reimbursed_this_month numeric(14,2) DEFAULT 0 NOT NULL,
    net_position numeric(14,2) DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.business_services (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    directory_id uuid NOT NULL,
    name text NOT NULL,
    description text,
    price numeric(10,2),
    currency text DEFAULT 'USD'::text NOT NULL,
    duration_minutes integer,
    category text,
    is_active boolean DEFAULT true NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.business_subscriptions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    tier_id uuid,
    status text DEFAULT 'active'::text,
    billing_cycle text DEFAULT 'monthly'::text,
    price_paid numeric(10,2),
    currency text DEFAULT 'USD'::text,
    start_date date NOT NULL,
    end_date date,
    auto_renew boolean DEFAULT true,
    stripe_subscription_id text,
    created_at timestamp with time zone DEFAULT now(),
    external_payment_ref text
);

CREATE TABLE IF NOT EXISTS public.business_transfer_events (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    transfer_id uuid NOT NULL,
    business_id uuid,
    actor_user_id uuid,
    actor_role character varying(32),
    event character varying(32) NOT NULL,
    from_status character varying(20),
    to_status character varying(20),
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.business_transfer_fees (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    transfer_id uuid NOT NULL,
    business_id uuid,
    payer_user_id uuid,
    payee_user_id uuid,
    amount_cents integer DEFAULT 0 NOT NULL,
    currency character varying(8) DEFAULT 'USD'::character varying NOT NULL,
    status character varying(20) DEFAULT 'payable'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    settled_at timestamp with time zone,
    CONSTRAINT business_transfer_fees_amount_check CHECK ((amount_cents >= 0)),
    CONSTRAINT business_transfer_fees_status_check CHECK (((status)::text = ANY ((ARRAY['payable'::character varying, 'settled'::character varying, 'waived'::character varying])::text[])))
);

CREATE TABLE IF NOT EXISTS public.business_transfers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    from_tenant_id uuid,
    from_user_id uuid,
    to_tenant_id uuid,
    to_user_id uuid,
    fee_cents integer DEFAULT 0 NOT NULL,
    currency character varying(8) DEFAULT 'USD'::character varying NOT NULL,
    fee_direction character varying(16) DEFAULT 'incoming'::character varying NOT NULL,
    host_stays boolean DEFAULT true NOT NULL,
    status character varying(20) DEFAULT 'pending'::character varying NOT NULL,
    notes text,
    requested_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    decided_at timestamp with time zone,
    to_email text,
    from_email text,
    target_directory_id uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT business_transfers_fee_check CHECK ((fee_cents >= 0)),
    CONSTRAINT business_transfers_fee_direction_check CHECK (((fee_direction)::text = ANY ((ARRAY['incoming'::character varying, 'outgoing'::character varying, 'platform'::character varying])::text[]))),
    CONSTRAINT business_transfers_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'accepted'::character varying, 'declined'::character varying, 'cancelled'::character varying])::text[])))
);

CREATE TABLE IF NOT EXISTS public.business_verifications (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    directory_id uuid,
    method text DEFAULT 'manual'::text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    verified_by uuid,
    verified_at timestamp with time zone,
    verification_doc_url text,
    notes text,
    expires_at timestamp with time zone,
    verified_data jsonb DEFAULT '{}'::jsonb,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.businesses (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    name character varying(255) NOT NULL,
    slug character varying(255) NOT NULL,
    description text,
    category_id uuid,
    address character varying(255),
    city character varying(100),
    state character varying(50),
    zip character varying(20),
    phone character varying(50),
    email character varying(255),
    website character varying(500),
    latitude double precision,
    longitude double precision,
    rating double precision DEFAULT 0,
    review_count integer DEFAULT 0,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    search_vector tsvector,
    images jsonb DEFAULT '[]'::jsonb,
    local_priority integer DEFAULT 3,
    business_type text DEFAULT 'local'::text,
    supplier_fields jsonb DEFAULT '{}'::jsonb,
    featured_deal_id uuid,
    featured_cta text DEFAULT 'Deal of the Week 🔥'::text,
    featured_product_id uuid,
    featured_product_cta text DEFAULT 'Featured Product'::text,
    enriched_at timestamp with time zone,
    city_slug character varying(100),
    logo_url text,
    cover_url text,
    status character varying(20) DEFAULT 'active'::character varying NOT NULL,
    claimed boolean DEFAULT false NOT NULL,
    verified boolean DEFAULT false NOT NULL,
    featured boolean DEFAULT false NOT NULL,
    owner_id uuid,
    lat double precision,
    lng double precision,
    is_franchise boolean DEFAULT false NOT NULL,
    coreswift_contact_id uuid,
    CONSTRAINT businesses_business_type_check CHECK ((business_type = ANY (ARRAY['local'::text, 'supplier'::text, 'distributor'::text, 'wholesaler'::text, 'farm'::text, 'association'::text, 'manufacturer'::text, 'chain'::text, 'other'::text])))
);

COMMENT ON COLUMN public.businesses.business_type IS 'Business taxonomy. Canonical list lives in src/business_types.rs::BUSINESS_TYPES; this CONSTRAINT, the /api/v1/b2b/register validation and the register UI all read from it.';

CREATE TABLE IF NOT EXISTS public.buying_group_deals (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    group_id uuid NOT NULL,
    title text NOT NULL,
    description text,
    supplier_business_id uuid NOT NULL,
    product_name text NOT NULL,
    normal_price numeric(10,2),
    group_price numeric(10,2) NOT NULL,
    min_quantity integer NOT NULL,
    current_quantity integer DEFAULT 0,
    unit text DEFAULT 'each'::text,
    deadline date,
    status text DEFAULT 'active'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.buying_group_members (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    group_id uuid NOT NULL,
    business_id uuid NOT NULL,
    role text DEFAULT 'member'::text NOT NULL,
    joined_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.buying_groups (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    description text,
    category text,
    founder_business_id uuid NOT NULL,
    status text DEFAULT 'recruiting'::text NOT NULL,
    member_count integer DEFAULT 1,
    min_members integer DEFAULT 2,
    max_members integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.call_logs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    caller_number text,
    called_number text,
    direction text DEFAULT 'inbound'::text,
    duration_seconds integer DEFAULT 0,
    call_status text DEFAULT 'missed'::text,
    recording_url text,
    transcription text,
    business_id uuid,
    directory_id uuid,
    lead_name text,
    lead_email text,
    lead_notes text,
    lead_status text DEFAULT 'new'::text,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.category_redeem_caps (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    category_name character varying(100) NOT NULL,
    max_redeem_percent integer DEFAULT 100 NOT NULL,
    description text
);

CREATE TABLE IF NOT EXISTS public.category_requests (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    category_id uuid NOT NULL,
    requested_by uuid,
    status text DEFAULT 'pending'::text NOT NULL,
    notes text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT category_requests_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'approved'::text, 'denied'::text])))
);

CREATE TABLE IF NOT EXISTS public.checkout_sessions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    provider_type text NOT NULL,
    provider_session_id text,
    purchasable_type text NOT NULL,
    purchasable_id uuid,
    business_id uuid NOT NULL,
    directory_id uuid,
    amount numeric(10,2) NOT NULL,
    currency text DEFAULT 'USD'::text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    webhook_received_at timestamp with time zone,
    webhook_event_id text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT checkout_sessions_purchasable_type_check CHECK ((purchasable_type = ANY (ARRAY['plan_subscription'::text, 'sponsored_listing'::text, 'ad_zone'::text, 'credits'::text]))),
    CONSTRAINT checkout_sessions_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'completed'::text, 'failed'::text, 'expired'::text, 'refunded'::text])))
);

CREATE TABLE IF NOT EXISTS public.city_pages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid NOT NULL,
    city_slug character varying(255) NOT NULL,
    city_name character varying(255) NOT NULL,
    state character varying(50),
    description text,
    hero_image_url text,
    meta_title character varying(255),
    meta_description text,
    is_active boolean DEFAULT true NOT NULL,
    display_order integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.city_plan_slots (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    city_slug text NOT NULL,
    plan_tier_id uuid NOT NULL,
    total_slots integer DEFAULT 10 NOT NULL,
    filled_slots integer DEFAULT 0 NOT NULL
);

CREATE TABLE IF NOT EXISTS public.city_priority (
    directory_slug text NOT NULL,
    city_name text NOT NULL,
    priority integer DEFAULT 1 NOT NULL
);

CREATE TABLE IF NOT EXISTS public.city_requests (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    city_name text NOT NULL,
    state text DEFAULT 'FL'::text NOT NULL,
    email text,
    votes integer DEFAULT 1 NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    directory_id uuid,
    processed_at timestamp with time zone
);

CREATE TABLE IF NOT EXISTS public.claim_offers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    listing_id uuid NOT NULL,
    offer_type character varying(50) DEFAULT 'promo_code'::character varying NOT NULL,
    offer_title character varying(255) NOT NULL,
    offer_description text,
    promo_code character varying(100),
    coupon_image_url text,
    redemption_url text,
    redemption_phone character varying(50),
    discount_value character varying(100),
    expires_at timestamp with time zone,
    terms_conditions text,
    is_active boolean DEFAULT true NOT NULL,
    max_claims integer,
    current_claims integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.claimed_businesses (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid,
    owner_email text NOT NULL,
    owner_name text,
    owner_phone text,
    verification_method text DEFAULT 'email'::text,
    verified_at timestamp with time zone,
    is_active boolean DEFAULT true,
    dashboard_password_hash text,
    last_dashboard_login timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    user_id uuid,
    subscription_id uuid,
    visitor_account_id uuid
);

COMMENT ON COLUMN public.claimed_businesses.visitor_account_id IS 'Links to visitor_accounts.id for supplier portal registrations. Mutually exclusive with user_id in practice.';

CREATE TABLE IF NOT EXISTS public.community_events (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    business_id uuid,
    title text NOT NULL,
    description text,
    event_date timestamp with time zone NOT NULL,
    end_date timestamp with time zone,
    location text,
    address text,
    image_url text,
    category text,
    status text DEFAULT 'active'::text NOT NULL,
    max_attendees integer,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    zaarhub_featured boolean DEFAULT false,
    source_provider_id uuid,
    source_event_id text,
    url text,
    loyalty_program_id uuid,
    event_type text DEFAULT 'general'::text
);

CREATE TABLE IF NOT EXISTS public.connected_services (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_id uuid NOT NULL,
    service text NOT NULL,
    api_key_encrypted text,
    is_active boolean DEFAULT true NOT NULL,
    expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT connected_services_api_key_encrypted_check CHECK (((api_key_encrypted IS NULL) OR (api_key_encrypted = ''::text) OR (api_key_encrypted ~~ 'enc:v1:%'::text))),
    CONSTRAINT connected_services_service_check CHECK ((service = 'coreswift'::text))
);

CREATE TABLE IF NOT EXISTS public.content_queue (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    queue_type text NOT NULL,
    directory_id uuid,
    keyword text NOT NULL,
    template_id uuid,
    merge_fields jsonb,
    scheduled_for timestamp with time zone NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    retry_count integer DEFAULT 0,
    error_message text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    CONSTRAINT content_queue_queue_type_check CHECK ((queue_type = ANY (ARRAY['trap_door'::text, 'blog'::text]))),
    CONSTRAINT content_queue_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'generating'::text, 'completed'::text, 'failed'::text, 'cancelled'::text])))
);

CREATE TABLE IF NOT EXISTS public.content_research (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    topic_id uuid NOT NULL,
    directory_id uuid,
    question text NOT NULL,
    source_url text,
    source_domain text,
    entry_kind text,
    is_used boolean DEFAULT false NOT NULL,
    used_as_keyword boolean DEFAULT false NOT NULL,
    drafted_post_id uuid,
    freshness_score double precision,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.content_topics (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    service_id uuid,
    location_id uuid,
    title text DEFAULT ''::text NOT NULL,
    format_template text,
    target_keyword text,
    status text DEFAULT 'suggested'::text,
    scheduled_date timestamp with time zone,
    word_count_target integer DEFAULT 1000,
    assigned_author_id uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    name text DEFAULT ''::text NOT NULL,
    description text,
    keywords jsonb DEFAULT '[]'::jsonb NOT NULL,
    search_phrase text,
    question_count integer DEFAULT 0 NOT NULL,
    last_researched timestamp with time zone,
    CONSTRAINT content_topics_status_check CHECK ((status = ANY (ARRAY['suggested'::text, 'scheduled'::text, 'in_progress'::text, 'in_review'::text, 'published'::text])))
);

CREATE TABLE IF NOT EXISTS public.crm_contacts (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    first_name text,
    last_name text,
    email text,
    phone text,
    company text,
    "position" text,
    directory_id uuid,
    status text DEFAULT 'lead'::text,
    tags text[],
    notes text,
    source text DEFAULT 'manual'::text,
    assigned_to text,
    last_contacted_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.crm_deal_records (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    contact_id uuid,
    value numeric(12,2),
    currency text DEFAULT 'USD'::text,
    pipeline_id uuid,
    stage text,
    status text DEFAULT 'open'::text,
    directory_id uuid,
    expected_close_date date,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.crm_pipelines (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    stages jsonb DEFAULT '[]'::jsonb,
    directory_id uuid,
    default_pipeline boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.data_enrichment_logs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid,
    directory_id uuid,
    source text NOT NULL,
    enrichment_type text NOT NULL,
    data_before jsonb,
    data_after jsonb,
    confidence double precision DEFAULT 1.0,
    status text DEFAULT 'completed'::text NOT NULL,
    error_message text,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.deal_claims (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    deal_id uuid NOT NULL,
    visitor_name character varying(255) NOT NULL,
    visitor_email character varying(255) NOT NULL,
    visitor_phone character varying(50),
    claim_code character varying(20) NOT NULL,
    claimed_at timestamp with time zone DEFAULT now() NOT NULL,
    redeemed_at timestamp with time zone,
    notes text
);

CREATE TABLE IF NOT EXISTS public.deal_redemptions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    deal_id uuid NOT NULL,
    visitor_id uuid,
    business_id uuid,
    redemption_code text NOT NULL,
    status text DEFAULT 'active'::text,
    used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    CONSTRAINT deal_redemptions_status_check CHECK ((status = ANY (ARRAY['active'::text, 'used'::text, 'expired'::text, 'cancelled'::text])))
);

CREATE TABLE IF NOT EXISTS public.deal_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    description text,
    config jsonb DEFAULT '{}'::jsonb,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.deals (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    description text,
    original_price text,
    deal_price text,
    discount_percent integer,
    currency text DEFAULT 'USD'::text,
    image_url text,
    terms text,
    redemption_limit integer,
    redemption_count integer DEFAULT 0,
    status text DEFAULT 'active'::text,
    directory_id uuid,
    business_id uuid NOT NULL,
    start_date timestamp with time zone,
    end_date timestamp with time zone,
    featured boolean DEFAULT false,
    deal_type text DEFAULT 'coupon'::text,
    coupon_code text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    created_by_owner boolean DEFAULT false,
    page_template text DEFAULT 'classic'::text,
    accent_color text DEFAULT '#f27f2f'::text,
    cta_color text DEFAULT '#f27f2f'::text,
    cta_text text DEFAULT 'Claim This Deal'::text,
    show_timer boolean DEFAULT true,
    show_qr boolean DEFAULT true,
    fine_print text,
    gallery_images jsonb DEFAULT '[]'::jsonb,
    rotation_order integer DEFAULT 0,
    rotation_schedule text,
    next_rotation_at timestamp with time zone,
    last_rotated_at timestamp with time zone,
    zaarhub_featured boolean DEFAULT false,
    discount_type character varying(20) DEFAULT 'percentage'::character varying NOT NULL,
    discount_value numeric(10,2),
    max_claims integer,
    claims_count integer DEFAULT 0 NOT NULL,
    starts_at timestamp with time zone,
    expires_at timestamp with time zone,
    is_active boolean DEFAULT true NOT NULL,
    deal_price_numeric numeric(10,2),
    highlights jsonb DEFAULT '[]'::jsonb,
    premium_features boolean DEFAULT false NOT NULL,
    redemption_type character varying(20) DEFAULT 'code'::character varying NOT NULL,
    booking_url text,
    per_user_limit integer,
    loyalty_program_id uuid,
    points_required integer DEFAULT 0,
    CONSTRAINT deals_page_template_check CHECK ((page_template = ANY (ARRAY['classic'::text, 'modern'::text, 'bold'::text, 'minimal'::text, 'service'::text, 'ecommerce'::text, 'event'::text]))),
    CONSTRAINT deals_redemption_type_check CHECK (((redemption_type)::text = ANY (ARRAY[('code'::character varying)::text, ('qr'::character varying)::text, ('wallet'::character varying)::text, ('booking'::character varying)::text]))),
    CONSTRAINT deals_rotation_schedule_check CHECK ((rotation_schedule = ANY (ARRAY['none'::text, 'daily'::text, 'weekly'::text, 'biweekly'::text, 'monthly'::text])))
);

CREATE TABLE IF NOT EXISTS public.demand_analytics_settings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    window_options_days integer[] DEFAULT '{7,30,90,180,365}'::integer[] NOT NULL,
    default_window_days integer DEFAULT 90 NOT NULL,
    bucket_size character varying(24) DEFAULT 'hour'::character varying NOT NULL,
    top_categories integer DEFAULT 5 NOT NULL,
    export_row_limit integer DEFAULT 5000 NOT NULL,
    rollup_modes text[] DEFAULT ARRAY['matrix'::text, 'rollups'::text, 'supply'::text] NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    unmet_min_searches integer DEFAULT 2 NOT NULL,
    unmet_queue_limit integer DEFAULT 25 NOT NULL,
    CONSTRAINT demand_analytics_settings_bucket_check CHECK (((bucket_size)::text = ANY ((ARRAY['hour'::character varying, 'day_of_week'::character varying, 'day'::character varying, 'week'::character varying])::text[]))),
    CONSTRAINT demand_analytics_settings_unmet_check CHECK (((unmet_min_searches > 0) AND (unmet_queue_limit > 0))),
    CONSTRAINT demand_analytics_settings_window_check CHECK (((default_window_days > 0) AND (top_categories > 0) AND (export_row_limit > 0)))
);

CREATE TABLE IF NOT EXISTS public.directories (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name character varying(255) NOT NULL,
    slug character varying(100) NOT NULL,
    description text,
    status character varying(20) DEFAULT 'draft'::character varying,
    owner_id uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    template character varying(64) DEFAULT 'local-business'::character varying,
    color_scheme jsonb DEFAULT '{"text": "#1e293b", "muted": "#94a3b8", "accent": "#f59e0b", "border": "#e2e8f0", "heading": "#0f172a", "primary": "#2563eb", "secondary": "#64748b", "background": "#ffffff"}'::jsonb,
    city text,
    template_config jsonb,
    connected_directory_ids jsonb DEFAULT '[]'::jsonb,
    api_config jsonb DEFAULT '{}'::jsonb,
    page_slug_pattern text DEFAULT '/{service}/{city}'::text,
    ai_provider text,
    ai_model text,
    ai_api_key_id uuid,
    ai_prompt_template text,
    ai_word_count_min integer DEFAULT 800,
    ai_word_count_max integer DEFAULT 1200,
    google_maps_api_key text,
    internal_linking_enabled boolean DEFAULT true,
    internal_linking_logic text DEFAULT 'same_category_city'::text,
    network_id uuid,
    url_type character varying(20) DEFAULT 'standalone'::character varying,
    url_value character varying(255),
    custom_domain character varying(255),
    tracking_enabled boolean DEFAULT true,
    head_injection text DEFAULT ''::text,
    body_injection text DEFAULT ''::text,
    footer_injection text DEFAULT ''::text,
    coreswift_tenant_id uuid,
    coreswift_key_prefix text,
    coreswift_list_id_claimed uuid,
    coreswift_list_id_newsletter uuid,
    email_signature_html text,
    email_signature_text text,
    coreswift_list_id_sponsors uuid,
    booking_calendar_slug text,
    feature_config jsonb DEFAULT '{"deals": true, "blogging": true, "gamification": false, "b2b_marketplace": false, "community_posts": true, "visitor_accounts": true}'::jsonb,
    zaarhub_config jsonb DEFAULT '{"show_deals": true, "show_events": true, "show_reviews": true, "show_activity": true, "network_visible": true, "homepage_featured": false, "featured_image_url": null, "homepage_hero_title": null, "homepage_hero_subtitle": null}'::jsonb,
    state character varying(50) DEFAULT 'FL'::character varying,
    display_order integer DEFAULT 0,
    coreswift_base_url character varying(512),
    coreswift_list_id_users uuid,
    coreswift_list_id_businesses uuid,
    coreswift_list_id_suppliers uuid,
    coreswift_personal_key_encrypted bytea,
    CONSTRAINT directories_internal_linking_logic_check CHECK ((internal_linking_logic = ANY (ARRAY['same_category'::text, 'same_city'::text, 'same_category_city'::text, 'both'::text])))
);

CREATE TABLE IF NOT EXISTS public.directory_branding (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    primary_color character varying(7) DEFAULT '#3B82F6'::character varying,
    secondary_color character varying(7) DEFAULT '#10B981'::character varying,
    accent_color character varying(7) DEFAULT '#F59E0B'::character varying,
    background_color character varying(7) DEFAULT '#FFFFFF'::character varying,
    text_color character varying(7) DEFAULT '#1F2937'::character varying,
    heading_color character varying(7) DEFAULT '#111827'::character varying,
    link_color character varying(7) DEFAULT '#3B82F6'::character varying,
    button_background character varying(7) DEFAULT '#3B82F6'::character varying,
    button_text character varying(7) DEFAULT '#FFFFFF'::character varying,
    heading_font character varying(100) DEFAULT 'Inter'::character varying,
    body_font character varying(100) DEFAULT 'Inter'::character varying,
    logo_url text,
    favicon_url text,
    meta_title_template character varying(255) DEFAULT '{business} | {directory}'::character varying,
    meta_description_template text,
    extracted_from_url text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.directory_categories (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    icon text DEFAULT ''::text,
    group_name text DEFAULT ''::text,
    created_at timestamp with time zone DEFAULT now(),
    directory_id uuid,
    parent_id uuid,
    sort_order integer DEFAULT 0
);

CREATE TABLE IF NOT EXISTS public.directory_email_settings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    smtp_host text DEFAULT ''::text NOT NULL,
    smtp_port integer DEFAULT 587 NOT NULL,
    smtp_username text DEFAULT ''::text NOT NULL,
    smtp_password text DEFAULT ''::text NOT NULL,
    smtp_encryption text DEFAULT 'tls'::text NOT NULL,
    from_name text DEFAULT ''::text NOT NULL,
    from_email text DEFAULT ''::text NOT NULL,
    reply_to text DEFAULT ''::text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    transport text DEFAULT 'smtp'::text NOT NULL,
    CONSTRAINT directory_email_settings_transport_check CHECK ((transport = ANY (ARRAY['smtp'::text, 'mailgun'::text, 'sendgrid'::text, 'sendiio'::text])))
);

CREATE TABLE IF NOT EXISTS public.directory_events (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    event_type text NOT NULL,
    entity_type text NOT NULL,
    entity_id uuid,
    directory_id uuid,
    tenant_id uuid,
    actor_id uuid,
    data jsonb DEFAULT '{}'::jsonb,
    metadata jsonb DEFAULT '{}'::jsonb,
    processed boolean DEFAULT false,
    n8n_webhook_sent boolean DEFAULT false,
    n8n_webhook_failed boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.directory_locations (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    state text,
    region text,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.directory_notifications (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    message text NOT NULL,
    link_text text,
    link_url text,
    notification_type text DEFAULT 'info'::text NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    starts_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.directory_services (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    description text,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.directory_surveys (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    enabled boolean DEFAULT false NOT NULL,
    title text DEFAULT 'Help us personalize your experience'::text NOT NULL,
    description text DEFAULT ''::text,
    questions jsonb DEFAULT '[]'::jsonb NOT NULL,
    completion_tags jsonb DEFAULT '[]'::jsonb NOT NULL,
    trigger_event text DEFAULT 'first_visit'::text NOT NULL,
    required boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    audience text DEFAULT 'customer'::text NOT NULL,
    status text DEFAULT 'draft'::text NOT NULL,
    reward_units integer DEFAULT 0 NOT NULL,
    network_id uuid,
    published_at timestamp with time zone,
    CONSTRAINT directory_surveys_audience_check CHECK ((audience = ANY (ARRAY['customer'::text, 'supplier'::text, 'business'::text]))),
    CONSTRAINT directory_surveys_status_check CHECK ((status = ANY (ARRAY['draft'::text, 'published'::text])))
);

COMMENT ON COLUMN public.directory_surveys.questions IS 'JSONB array of {id,type,label,help_text,required,options[],scale_min,scale_max,order}. Types: short_text,long_text,single_choice,multiple_choice,dropdown,number,yes_no,rating,date.';

COMMENT ON COLUMN public.directory_surveys.audience IS 'Who the questionnaire is for: customer | supplier | business. Authored per directory in the admin panel.';

COMMENT ON COLUMN public.directory_surveys.status IS 'draft | published. Only a published questionnaire is served to the public; draft never reaches a visitor.';

COMMENT ON COLUMN public.directory_surveys.reward_units IS 'Native currency units credited on completion (100 units = US$1). 0 = answers are stored but nothing is earned.';

CREATE TABLE IF NOT EXISTS public.directory_tiers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    tier_slug text NOT NULL,
    tier_name text DEFAULT 'Free'::text NOT NULL,
    is_active boolean DEFAULT true,
    started_at timestamp with time zone DEFAULT now(),
    expires_at timestamp with time zone,
    stripe_subscription_id text,
    stripe_customer_id text,
    metadata jsonb DEFAULT '{}'::jsonb,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    external_plan_id text,
    external_checkout_url text,
    plan_tier_id uuid
);

CREATE TABLE IF NOT EXISTS public.discovery_queue (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    place_id text,
    name text NOT NULL,
    address text,
    city text,
    phone text,
    website text,
    rating double precision DEFAULT 0,
    review_count integer DEFAULT 0,
    types text[] DEFAULT '{}'::text[] NOT NULL,
    mapped_category_id uuid,
    mapped_category text,
    is_franchise boolean DEFAULT false NOT NULL,
    is_duplicate boolean DEFAULT false NOT NULL,
    selected boolean DEFAULT false NOT NULL,
    status text DEFAULT 'queued'::text NOT NULL,
    raw jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    source text DEFAULT 'admin_search'::text NOT NULL,
    demand_volume integer,
    demand_area text
);

CREATE TABLE IF NOT EXISTS public.domain_mappings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    domain character varying(255) NOT NULL,
    type character varying(20) DEFAULT 'subfolder'::character varying NOT NULL,
    status character varying(20) DEFAULT 'pending'::character varying,
    ssl_enabled boolean DEFAULT false,
    cloudflare_record_id character varying(255),
    dns_records jsonb DEFAULT '{}'::jsonb,
    verification_token character varying(255),
    auto_configured boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    url_path text DEFAULT ''::text NOT NULL,
    live_status text,
    last_checked_at timestamp with time zone,
    last_check_detail text
);

CREATE TABLE IF NOT EXISTS public.email_campaigns (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    template_id uuid,
    recipient_filter jsonb,
    sent_count integer DEFAULT 0,
    opened_count integer DEFAULT 0,
    status text DEFAULT 'draft'::text,
    scheduled_at timestamp with time zone,
    sent_at timestamp with time zone,
    directory_id uuid,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.email_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    subject text NOT NULL,
    body text NOT NULL,
    variables text[],
    category text DEFAULT 'general'::text,
    directory_id uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    body_text text
);

CREATE TABLE IF NOT EXISTS public.enrichment_settings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    is_enabled boolean DEFAULT false NOT NULL,
    cadence_hours integer DEFAULT 24 NOT NULL,
    batch_size integer DEFAULT 25 NOT NULL,
    provider character varying(64),
    last_run_at timestamp with time zone,
    last_status character varying(32),
    next_run_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT enrichment_settings_batch_check CHECK (((batch_size >= 1) AND (batch_size <= 500))),
    CONSTRAINT enrichment_settings_cadence_check CHECK ((cadence_hours >= 1))
);

CREATE TABLE IF NOT EXISTS public.event_providers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    provider_type text NOT NULL,
    api_key text,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    last_sync_at timestamp with time zone,
    last_sync_status text,
    last_error text,
    events_synced integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT event_providers_provider_type_check CHECK ((provider_type = ANY (ARRAY['eventbrite'::text, 'meetup'::text, 'ics_feed'::text, 'n8n_webhook'::text])))
);

CREATE TABLE IF NOT EXISTS public.event_rsvps (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    event_id uuid NOT NULL,
    visitor_account_id uuid NOT NULL,
    status text DEFAULT 'going'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.export_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    entity_type text NOT NULL,
    fields jsonb NOT NULL,
    directory_id uuid,
    delimiter text DEFAULT chr(44),
    include_header boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.google_places_cache (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    query text NOT NULL,
    place_id text,
    name text,
    formatted_address text,
    phone text,
    website text,
    latitude double precision,
    longitude double precision,
    rating double precision,
    user_ratings_total integer,
    types text[],
    photos text[],
    opening_hours jsonb,
    place_details jsonb,
    cached_at timestamp with time zone DEFAULT now(),
    expires_at timestamp with time zone DEFAULT (now() + '7 days'::interval)
);

CREATE TABLE IF NOT EXISTS public.grandfathered_pricing (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    service_key character varying(100) NOT NULL,
    price_monthly numeric(10,2),
    price_yearly numeric(10,2),
    price_one_time numeric(10,2),
    expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.group_deal_commitments (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    deal_id uuid NOT NULL,
    business_id uuid NOT NULL,
    quantity integer NOT NULL,
    total_amount numeric(10,2),
    status text DEFAULT 'committed'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.homepage_sections (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    directory_id uuid,
    section_type character varying(50) NOT NULL,
    sort_order integer DEFAULT 0,
    title character varying(255),
    subtitle text,
    content text,
    cta_text character varying(100),
    cta_url character varying(500),
    image_url text,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT homepage_owner_check CHECK ((((network_id IS NOT NULL) AND (directory_id IS NULL)) OR ((network_id IS NULL) AND (directory_id IS NOT NULL))))
);

CREATE TABLE IF NOT EXISTS public.import_logs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    entity_type text NOT NULL,
    filename text,
    rows_total integer DEFAULT 0,
    rows_success integer DEFAULT 0,
    rows_failed integer DEFAULT 0,
    errors jsonb DEFAULT '[]'::jsonb,
    directory_id uuid,
    status text DEFAULT 'pending'::text,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.integration_configs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    provider text NOT NULL,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    enabled boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.integration_provider_presets (
    key text NOT NULL,
    name text NOT NULL,
    base_url text DEFAULT ''::text NOT NULL,
    docs_url text,
    sort_order integer DEFAULT 0 NOT NULL,
    is_active boolean DEFAULT true NOT NULL
);

CREATE TABLE IF NOT EXISTS public.landing_pages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    slug text NOT NULL,
    directory_id uuid,
    hero_title text,
    hero_subtitle text,
    hero_cta_text text,
    hero_cta_url text,
    features jsonb DEFAULT '[]'::jsonb,
    testimonials jsonb DEFAULT '[]'::jsonb,
    faq jsonb DEFAULT '[]'::jsonb,
    seo_title text,
    seo_description text,
    published boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.lead_share_transactions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    lead_id uuid NOT NULL,
    from_business_id uuid NOT NULL,
    to_business_id uuid NOT NULL,
    status text DEFAULT 'transferred'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.legal_pages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    page_type text DEFAULT 'custom'::text NOT NULL,
    content text NOT NULL,
    published boolean DEFAULT true,
    is_global boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.link_clicks (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    link_id uuid NOT NULL,
    contact_id uuid,
    ip_address inet,
    user_agent text,
    referer text,
    country text,
    city text,
    device_type text,
    clicked_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.loyalty_activity (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    member_id uuid NOT NULL,
    activity_type character varying(50) NOT NULL,
    description text,
    points_earned bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.loyalty_checkins (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    member_id uuid NOT NULL,
    points_awarded integer NOT NULL,
    method text,
    checked_in_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.loyalty_enrollments (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    program_id uuid NOT NULL,
    entity_type text NOT NULL,
    entity_id uuid NOT NULL,
    enrolled_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.loyalty_members (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    program_id uuid NOT NULL,
    visitor_account_id uuid NOT NULL,
    points_balance integer DEFAULT 0,
    lifetime_points integer DEFAULT 0,
    tier_id uuid,
    current_streak integer DEFAULT 0,
    longest_streak integer DEFAULT 0,
    last_activity_date timestamp with time zone,
    birthday date,
    referral_code character varying(50),
    total_referrals integer DEFAULT 0,
    qr_code text,
    qr_code_generated_at timestamp with time zone,
    member_since timestamp with time zone DEFAULT now(),
    last_checkin_at timestamp with time zone,
    network_id uuid
);

CREATE TABLE IF NOT EXISTS public.loyalty_milestones (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    loyalty_program_id uuid NOT NULL,
    name character varying(200) NOT NULL,
    trigger_type character varying(50) NOT NULL,
    trigger_value bigint DEFAULT 0 NOT NULL,
    bonus_points bigint DEFAULT 0 NOT NULL,
    bonus_reward_id uuid,
    once_per_member boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.loyalty_milestones_completed (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    member_id uuid NOT NULL,
    milestone_id uuid NOT NULL,
    points_awarded bigint DEFAULT 0 NOT NULL,
    completed_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.loyalty_programs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    name text NOT NULL,
    recognition_method text DEFAULT 'both'::text,
    points_per_checkin integer DEFAULT 10,
    max_checkins_per_day integer DEFAULT 1,
    point_decay_days integer,
    points_expire_days integer DEFAULT 365,
    currency_name text DEFAULT 'Points'::text,
    currency_icon text DEFAULT '⭐'::text,
    currency_color text DEFAULT '#0d9488'::text,
    points_per_visit integer DEFAULT 5,
    tiers_enabled boolean DEFAULT false,
    milestones_enabled boolean DEFAULT false,
    streak_enabled boolean DEFAULT false,
    streak_bonus integer DEFAULT 0,
    streak_days integer DEFAULT 7,
    referral_bonus integer DEFAULT 0,
    birthday_bonus integer DEFAULT 0,
    social_share_points integer DEFAULT 0,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    network_id uuid,
    points_per_redemption integer DEFAULT 0 NOT NULL,
    earn_rate double precision DEFAULT 1 NOT NULL,
    redemption_cap_pct integer DEFAULT 10 NOT NULL,
    min_redeem_balance integer DEFAULT 100 NOT NULL,
    exclude_free_items boolean DEFAULT true NOT NULL
);

COMMENT ON COLUMN public.loyalty_programs.points_per_redemption IS 'Loyalty points credited to a member when they redeem a deal for this programme. 0 = disabled.';

COMMENT ON COLUMN public.loyalty_programs.earn_rate IS 'Currency units credited per $1 of earnable spend. 0 = earning disabled. Default 1.';

COMMENT ON COLUMN public.loyalty_programs.redemption_cap_pct IS 'Maximum percentage of a bill that a member may settle with the programme currency (0-100). Default 10.';

COMMENT ON COLUMN public.loyalty_programs.min_redeem_balance IS 'Balance a member must hold before they may redeem. 100 units = $1, so the default 100 = $1.';

COMMENT ON COLUMN public.loyalty_programs.exclude_free_items IS 'When true, free or fully-discounted items earn no currency. Default true.';

CREATE TABLE IF NOT EXISTS public.loyalty_reward_tiers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    program_id uuid NOT NULL,
    name text NOT NULL,
    points_required integer NOT NULL,
    requires_approval boolean DEFAULT false,
    reward_tag text NOT NULL,
    marketing_boost jsonb,
    sort_order integer DEFAULT 0
);

CREATE TABLE IF NOT EXISTS public.loyalty_rewards_earned (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    member_id uuid NOT NULL,
    tier_id uuid,
    status text DEFAULT 'pending'::text,
    earned_at timestamp with time zone DEFAULT now(),
    approved_by uuid,
    fulfilled_at timestamp with time zone
);

CREATE TABLE IF NOT EXISTS public.loyalty_scans (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    member_id uuid NOT NULL,
    program_id uuid NOT NULL,
    business_id uuid,
    scan_type text DEFAULT 'checkin'::text NOT NULL,
    points_awarded integer DEFAULT 0,
    metadata jsonb DEFAULT '{}'::jsonb,
    scanned_at timestamp with time zone DEFAULT now(),
    business_name text,
    points_balance integer,
    deal_applied text,
    transaction_amount numeric(14,2),
    business_category text,
    clearinghouse_processed boolean DEFAULT false,
    cleared_at timestamp with time zone
);

CREATE TABLE IF NOT EXISTS public.loyalty_tiers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    loyalty_program_id uuid NOT NULL,
    name character varying(100) NOT NULL,
    min_points bigint DEFAULT 0 NOT NULL,
    color character varying(7) DEFAULT '#6B7280'::character varying NOT NULL,
    perks jsonb DEFAULT '[]'::jsonb,
    multiplier numeric(5,2) DEFAULT 1.0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.network_branding (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid NOT NULL,
    logo_url text,
    logo_footer_url text,
    favicon_url text,
    primary_color character varying(7) DEFAULT '#2563eb'::character varying,
    secondary_color character varying(7) DEFAULT '#64748b'::character varying,
    accent_color character varying(7) DEFAULT '#f59e0b'::character varying,
    background_color character varying(7) DEFAULT '#ffffff'::character varying,
    text_color character varying(7) DEFAULT '#1e293b'::character varying,
    heading_color character varying(7) DEFAULT '#0f172a'::character varying,
    heading_font character varying(100) DEFAULT 'Inter'::character varying,
    body_font character varying(100) DEFAULT 'Inter'::character varying,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.networks (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name character varying(255) NOT NULL,
    slug character varying(100) NOT NULL,
    description text,
    root_domain character varying(255),
    status character varying(20) DEFAULT 'active'::character varying,
    owner_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    coreswift_tenant_id uuid,
    coreswift_key_prefix text,
    coreswift_list_id_sponsors uuid,
    coreswift_list_id_claimed uuid,
    coreswift_list_id_newsletter uuid,
    coreswift_personal_key_encrypted bytea,
    coreswift_base_url character varying(512),
    coreswift_list_id_users uuid,
    coreswift_list_id_businesses uuid,
    coreswift_list_id_suppliers uuid
);

CREATE TABLE IF NOT EXISTS public.newsletter_digests (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    title text NOT NULL,
    body text,
    source_post_ids uuid[] DEFAULT '{}'::uuid[],
    status text DEFAULT 'draft'::text,
    scheduled_at timestamp with time zone,
    sent_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.newsletter_queue (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    title text NOT NULL,
    intro_text text,
    include_blog boolean DEFAULT true,
    include_deals boolean DEFAULT true,
    manual_sections jsonb DEFAULT '[]'::jsonb,
    scheduled_at timestamp with time zone,
    sent_at timestamp with time zone,
    status text DEFAULT 'draft'::text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.newsletter_subscribers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    email text NOT NULL,
    name text DEFAULT ''::text,
    status text DEFAULT 'active'::text NOT NULL,
    subscribed_at timestamp with time zone DEFAULT now(),
    unsubscribed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.offer_claims (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    offer_id uuid NOT NULL,
    visitor_id character varying(255) NOT NULL,
    email character varying(255),
    phone character varying(50),
    promo_code_revealed character varying(100),
    claimed_at timestamp with time zone DEFAULT now() NOT NULL,
    redeemed boolean DEFAULT false NOT NULL,
    redeemed_at timestamp with time zone,
    ip_address character varying(50),
    user_agent text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.password_resets (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_id uuid NOT NULL,
    token text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.pay_per_call (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid,
    directory_id uuid,
    call_sid character varying(255),
    caller_number character varying(50),
    duration_seconds integer DEFAULT 0,
    cost_cents integer DEFAULT 0,
    status character varying(20) DEFAULT 'pending'::character varying,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.payment_providers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    provider_type text NOT NULL,
    label text DEFAULT ''::text NOT NULL,
    is_active boolean DEFAULT false NOT NULL,
    api_key_encrypted text,
    webhook_secret_encrypted text,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    publishable_key text,
    webhook_path text,
    is_test_mode boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT payment_providers_api_key_encrypted_check CHECK (((api_key_encrypted IS NULL) OR (api_key_encrypted = ''::text) OR (api_key_encrypted ~~ 'enc:v1:%'::text))),
    CONSTRAINT payment_providers_provider_type_check CHECK ((provider_type = ANY (ARRAY['stripe'::text, 'paypal'::text, 'square'::text, 'paddle'::text]))),
    CONSTRAINT payment_providers_webhook_secret_encrypted_check CHECK (((webhook_secret_encrypted IS NULL) OR (webhook_secret_encrypted = ''::text) OR (webhook_secret_encrypted ~~ 'enc:v1:%'::text)))
);

CREATE TABLE IF NOT EXISTS public.payment_webhook_events (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    provider_type text NOT NULL,
    event_type text,
    event_id text,
    raw_body jsonb,
    headers jsonb,
    status text DEFAULT 'received'::text NOT NULL,
    error_message text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT payment_webhook_events_status_check CHECK ((status = ANY (ARRAY['received'::text, 'processed'::text, 'failed'::text, 'ignored'::text, 'not_configured'::text, 'signature_failed'::text])))
);

CREATE TABLE IF NOT EXISTS public.plan_slot_bookings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    city_slug text NOT NULL,
    plan_tier_id uuid NOT NULL,
    business_id uuid,
    business_name text NOT NULL,
    contact_email text NOT NULL,
    start_date date NOT NULL,
    end_date date NOT NULL,
    slot_position integer NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    price_paid numeric(10,2),
    currency text DEFAULT 'USD'::text,
    coreswift_contact_id uuid,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    contact_phone text,
    metadata jsonb DEFAULT '{}'::jsonb
);

CREATE TABLE IF NOT EXISTS public.plan_tiers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    price_monthly numeric(10,2) DEFAULT 0,
    price_yearly numeric(10,2) DEFAULT 0,
    max_listings integer DEFAULT '-1'::integer,
    max_deals integer DEFAULT 0,
    max_photos integer DEFAULT 5,
    has_reviews boolean DEFAULT true,
    has_analytics boolean DEFAULT false,
    has_crm boolean DEFAULT false,
    has_email boolean DEFAULT false,
    has_call_tracking boolean DEFAULT false,
    has_import_export boolean DEFAULT false,
    has_api_access boolean DEFAULT false,
    featured_listing boolean DEFAULT false,
    description text,
    created_at timestamp with time zone DEFAULT now(),
    max_industries integer DEFAULT 1,
    plan_sales_page_url text,
    coreswift_tag text,
    coreswift_pipeline_stage text,
    slot_duration_days integer DEFAULT 30,
    max_slots_per_city integer,
    payment_provider character varying(64),
    thank_you_url character varying(512),
    max_active_deals integer DEFAULT 0,
    max_scheduled_rotations integer DEFAULT 0,
    allow_custom_branding boolean DEFAULT false,
    feature_access jsonb DEFAULT '{}'::jsonb,
    max_categories integer DEFAULT 1
);

CREATE TABLE IF NOT EXISTS public.point_issuance_log (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    issuing_business_id uuid,
    business_name text,
    member_id uuid,
    program_id uuid,
    scan_id uuid,
    points_issued integer DEFAULT 0 NOT NULL,
    bill_rate_cents integer DEFAULT 1 NOT NULL,
    total_billed_cents integer DEFAULT 0 NOT NULL,
    transaction_amount numeric(14,2),
    transaction_id text,
    issuance_type text DEFAULT 'purchase'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expired_at timestamp with time zone
);

CREATE TABLE IF NOT EXISTS public.point_redemption_log (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    redeeming_business_id uuid,
    business_name text,
    member_id uuid,
    program_id uuid,
    scan_id uuid,
    points_redeemed integer DEFAULT 0 NOT NULL,
    reimbursement_rate_cents integer DEFAULT 0 NOT NULL,
    total_reimbursement_cents integer DEFAULT 0 NOT NULL,
    transaction_amount numeric(14,2),
    max_redeem_percent integer,
    transaction_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.point_treasury (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    issuance_rate numeric(12,6) DEFAULT 0.010000 NOT NULL,
    redemption_rate numeric(12,6) DEFAULT 0.008000 NOT NULL,
    platform_spread_percent numeric(8,4) DEFAULT 20.0000 NOT NULL,
    minimum_float numeric(14,2) DEFAULT 100.00 NOT NULL,
    default_expiry_days integer DEFAULT 365 NOT NULL,
    total_points_issued bigint DEFAULT 0 NOT NULL,
    total_points_redeemed bigint DEFAULT 0 NOT NULL,
    total_revenue_collected numeric(14,2) DEFAULT 0 NOT NULL,
    total_reimbursements_paid numeric(14,2) DEFAULT 0 NOT NULL,
    outstanding_liability numeric(14,2) DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    currency character varying(8) DEFAULT 'USD'::character varying NOT NULL,
    minimum_payout_cents integer DEFAULT 0 NOT NULL,
    settlement_enabled boolean DEFAULT true NOT NULL,
    payment_provider character varying(32),
    cycle_day integer DEFAULT 1 NOT NULL,
    expiry_last_run_at timestamp with time zone
);

CREATE TABLE IF NOT EXISTS public.poll_votes (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    poll_id uuid NOT NULL,
    visitor_account_id uuid NOT NULL,
    option_index integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.polls (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    question text NOT NULL,
    options text[] DEFAULT '{}'::text[] NOT NULL,
    created_by uuid NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    starts_at timestamp with time zone DEFAULT now() NOT NULL,
    ends_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.price_bundles (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    network_id uuid,
    name character varying(255) NOT NULL,
    slug character varying(100) NOT NULL,
    description text,
    price_monthly numeric(10,2),
    price_yearly numeric(10,2),
    is_active boolean DEFAULT true,
    sort_order integer DEFAULT 0,
    is_featured boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.programmatic_pages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    service_id uuid,
    location_id uuid,
    slug text NOT NULL,
    title text,
    meta_title text,
    meta_description text,
    h1 text,
    content text,
    template_name text DEFAULT 'default'::text,
    status text DEFAULT 'draft'::text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    day_tags text[] DEFAULT '{}'::text[],
    time_tags text[] DEFAULT '{}'::text[],
    hour_slot text,
    impressions integer DEFAULT 0,
    clicks integer DEFAULT 0,
    conversions integer DEFAULT 0,
    mentioned_business_ids uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    CONSTRAINT programmatic_pages_status_check CHECK ((status = ANY (ARRAY['draft'::text, 'published'::text, 'archived'::text])))
);

CREATE TABLE IF NOT EXISTS public.provider_keys (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid NOT NULL,
    provider character varying(64) NOT NULL,
    api_key text NOT NULL,
    base_url character varying(512),
    metadata jsonb DEFAULT '{}'::jsonb,
    is_active boolean DEFAULT true,
    scope character varying(16) DEFAULT 'tenant'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    api_key_encrypted bytea,
    base_url_encrypted bytea,
    label text DEFAULT 'default'::text NOT NULL,
    is_default boolean DEFAULT false NOT NULL,
    network_id uuid,
    directory_id uuid,
    CONSTRAINT provider_keys_api_key_encrypted CHECK (((api_key = ''::text) OR (api_key ~~ 'enc:v1:%'::text))),
    CONSTRAINT provider_keys_single_scope CHECK ((NOT ((network_id IS NOT NULL) AND (directory_id IS NOT NULL))))
);

CREATE TABLE IF NOT EXISTS public.public_pages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    description text,
    original_price numeric(10,2),
    public_page_price numeric(10,2),
    discount_percent numeric(5,2),
    currency text DEFAULT 'USD'::text,
    image_url text,
    terms text,
    redemption_limit integer,
    redemption_count integer DEFAULT 0 NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    directory_id uuid,
    business_id uuid,
    start_date timestamp with time zone,
    end_date timestamp with time zone,
    featured boolean DEFAULT false NOT NULL,
    public_page_type text,
    coupon_code text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.public_themes (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    directory_id uuid,
    primary_color text DEFAULT '#2563eb'::text,
    secondary_color text DEFAULT '#1e40af'::text,
    header_style text DEFAULT 'gradient'::text,
    layout text DEFAULT 'grid'::text,
    show_search boolean DEFAULT true,
    show_categories boolean DEFAULT true,
    show_featured boolean DEFAULT true,
    items_per_page integer DEFAULT 12,
    custom_css text,
    custom_js text,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.referrals (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    referrer_type text NOT NULL,
    referrer_id uuid NOT NULL,
    referrer_email text,
    referee_type text NOT NULL,
    referee_id uuid,
    referee_email text,
    referee_name text,
    referral_code text NOT NULL,
    direction text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    zaarcash_earned integer DEFAULT 0,
    verified_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.reviews (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid,
    user_id uuid,
    rating integer NOT NULL,
    title character varying(255),
    content text,
    reviewer_name character varying(100),
    is_verified boolean DEFAULT false,
    created_at timestamp with time zone DEFAULT now(),
    reviewer_email text,
    status text DEFAULT 'pending'::text,
    featured boolean DEFAULT false,
    source text DEFAULT 'direct'::text,
    source_url text,
    directory_id uuid,
    updated_at timestamp with time zone DEFAULT now(),
    email_sent_at timestamp with time zone,
    CONSTRAINT reviews_rating_check CHECK (((rating >= 1) AND (rating <= 5)))
);

CREATE TABLE IF NOT EXISTS public.rfq_bids (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    rfq_id uuid NOT NULL,
    bidder_business_id uuid NOT NULL,
    amount numeric(10,2) NOT NULL,
    details text NOT NULL,
    delivery_timeline text,
    status text DEFAULT 'submitted'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.rfq_messages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    rfq_id uuid NOT NULL,
    sender_business_id uuid NOT NULL,
    message text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.rfqs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    description text NOT NULL,
    category text,
    quantity text,
    budget_min numeric(10,2),
    budget_max numeric(10,2),
    deadline date,
    delivery_location text,
    poster_business_id uuid NOT NULL,
    status text DEFAULT 'open'::text NOT NULL,
    urgency text DEFAULT 'standard'::text,
    is_public boolean DEFAULT true NOT NULL,
    awarded_to uuid,
    awarded_bid_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    closed_at timestamp with time zone,
    view_count integer DEFAULT 0
);

CREATE TABLE IF NOT EXISTS public.schema_config (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    schema_type text NOT NULL,
    enabled boolean DEFAULT true,
    config jsonb DEFAULT '{}'::jsonb,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.search_config (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    enable_fulltext boolean DEFAULT true,
    enable_filters boolean DEFAULT true,
    filter_fields jsonb DEFAULT '["category", "city", "state", "rating", "price"]'::jsonb,
    results_per_page integer DEFAULT 20,
    enable_location_search boolean DEFAULT false,
    default_radius_km integer DEFAULT 10,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.seo_fallback_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    page_type text NOT NULL,
    title_template text,
    description_template text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.seo_meta (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    page_type text NOT NULL,
    page_id uuid,
    title text,
    description text,
    keywords text,
    og_image text,
    og_title text,
    og_description text,
    schema_type text,
    custom_schema jsonb,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.service_bookings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    business_id uuid NOT NULL,
    visitor_account_id uuid NOT NULL,
    service_name text,
    description text,
    preferred_date timestamp with time zone,
    preferred_time text,
    contact_phone text,
    contact_email text,
    status text DEFAULT 'pending'::text NOT NULL,
    notes text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.service_prices (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    network_id uuid,
    service_key character varying(100) NOT NULL,
    price_monthly numeric(10,2),
    price_yearly numeric(10,2),
    price_one_time numeric(10,2),
    currency character varying(3) DEFAULT 'USD'::character varying,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    CONSTRAINT check_scope CHECK ((((directory_id IS NOT NULL) AND (network_id IS NULL)) OR ((directory_id IS NULL) AND (network_id IS NOT NULL)) OR ((directory_id IS NULL) AND (network_id IS NULL))))
);

CREATE TABLE IF NOT EXISTS public.settlement_invoices (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    run_id uuid NOT NULL,
    business_id uuid,
    business_name text,
    points_issued bigint DEFAULT 0 NOT NULL,
    rate_per_point numeric(12,6) DEFAULT 0.010000 NOT NULL,
    amount_cents numeric(14,2) DEFAULT 0 NOT NULL,
    currency character varying(8) DEFAULT 'USD'::character varying NOT NULL,
    status character varying(24) DEFAULT 'pending'::character varying NOT NULL,
    provider_ref text,
    due_date date,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    settled_at timestamp with time zone,
    CONSTRAINT settlement_invoices_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'pending_provider'::character varying, 'sent'::character varying, 'paid'::character varying, 'failed'::character varying, 'void'::character varying, 'below_minimum'::character varying])::text[])))
);

CREATE TABLE IF NOT EXISTS public.settlement_payouts (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    run_id uuid NOT NULL,
    business_id uuid,
    business_name text,
    points_redeemed bigint DEFAULT 0 NOT NULL,
    rate_per_point numeric(12,6) DEFAULT 0.008000 NOT NULL,
    amount_cents numeric(14,2) DEFAULT 0 NOT NULL,
    currency character varying(8) DEFAULT 'USD'::character varying NOT NULL,
    status character varying(24) DEFAULT 'pending'::character varying NOT NULL,
    provider character varying(32),
    provider_ref text,
    provider_message text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    paid_at timestamp with time zone,
    CONSTRAINT settlement_payouts_status_check CHECK (((status)::text = ANY ((ARRAY['pending'::character varying, 'pending_provider'::character varying, 'paid'::character varying, 'failed'::character varying, 'void'::character varying, 'below_minimum'::character varying])::text[])))
);

CREATE TABLE IF NOT EXISTS public.settlement_runs (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid NOT NULL,
    period_start date NOT NULL,
    period_end date NOT NULL,
    period_key character varying(16) NOT NULL,
    status character varying(24) DEFAULT 'draft'::character varying NOT NULL,
    currency character varying(8) DEFAULT 'USD'::character varying NOT NULL,
    rate_issue_per_point numeric(12,6) DEFAULT 0.010000 NOT NULL,
    rate_redeem_per_point numeric(12,6) DEFAULT 0.008000 NOT NULL,
    min_payout_cents integer DEFAULT 0 NOT NULL,
    cycle_day integer DEFAULT 1 NOT NULL,
    total_points_issued bigint DEFAULT 0 NOT NULL,
    total_points_redeemed bigint DEFAULT 0 NOT NULL,
    total_invoiced_cents numeric(14,2) DEFAULT 0 NOT NULL,
    total_payout_cents numeric(14,2) DEFAULT 0 NOT NULL,
    platform_spread_cents numeric(14,2) DEFAULT 0 NOT NULL,
    provider character varying(32),
    provider_ref text,
    provider_message text,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    completed_at timestamp with time zone,
    notes text,
    triggered_by character varying(16) DEFAULT 'manual'::character varying NOT NULL,
    statements_sent_at timestamp with time zone,
    statements_sent integer DEFAULT 0 NOT NULL,
    statements_skipped integer DEFAULT 0 NOT NULL,
    statements_message text,
    CONSTRAINT settlement_runs_status_check CHECK (((status)::text = ANY ((ARRAY['draft'::character varying, 'preview'::character varying, 'pending_provider'::character varying, 'processing'::character varying, 'completed'::character varying, 'failed'::character varying, 'cancelled'::character varying])::text[])))
);

CREATE OR REPLACE VIEW public.settlement_statements AS
 SELECT r.id AS run_id,
    r.network_id,
    r.period_key,
    r.period_start,
    r.period_end,
    r.status AS run_status,
    r.currency,
    b.business_id,
    COALESCE(i.business_name, p.business_name) AS business_name,
    COALESCE(i.points_issued, (0)::bigint) AS points_issued,
    COALESCE(i.amount_cents, (0)::numeric) AS invoiced_cents,
    COALESCE(p.points_redeemed, (0)::bigint) AS points_redeemed,
    COALESCE(p.amount_cents, (0)::numeric) AS reimbursed_cents,
    (COALESCE(i.amount_cents, (0)::numeric) - COALESCE(p.amount_cents, (0)::numeric)) AS net_position_cents,
    i.status AS invoice_status,
    p.status AS payout_status
   FROM (((public.settlement_runs r
     CROSS JOIN LATERAL ( SELECT settlement_invoices.business_id
           FROM public.settlement_invoices
          WHERE (settlement_invoices.run_id = r.id)
        UNION
         SELECT settlement_payouts.business_id
           FROM public.settlement_payouts
          WHERE (settlement_payouts.run_id = r.id)) b)
     LEFT JOIN public.settlement_invoices i ON (((i.run_id = r.id) AND (i.business_id = b.business_id))))
     LEFT JOIN public.settlement_payouts p ON (((p.run_id = r.id) AND (p.business_id = b.business_id))));

CREATE TABLE IF NOT EXISTS public.shared_leads (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    title text NOT NULL,
    description text NOT NULL,
    category text,
    location text,
    estimated_value numeric(10,2),
    source text,
    poster_business_id uuid NOT NULL,
    status text DEFAULT 'available'::text NOT NULL,
    claimed_by uuid,
    claimed_at timestamp with time zone,
    expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.sitemap_config (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid,
    auto_generate boolean DEFAULT true,
    priority numeric(2,1) DEFAULT 0.5,
    change_freq text DEFAULT 'weekly'::text,
    last_generated timestamp with time zone,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.sponsored_listings (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    business_id uuid NOT NULL,
    slot_position integer DEFAULT 1 NOT NULL,
    start_date date NOT NULL,
    end_date date NOT NULL,
    is_active boolean DEFAULT true,
    price_paid numeric(10,2) DEFAULT 0,
    currency text DEFAULT 'USD'::text,
    stripe_payment_intent_id text,
    featured boolean DEFAULT false,
    badge_text text,
    metadata jsonb DEFAULT '{}'::jsonb,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    external_payment_ref text
);

CREATE TABLE IF NOT EXISTS public.sponsors (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    business_id uuid NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    commission_rate numeric(5,2) DEFAULT 0,
    notes text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    CONSTRAINT sponsors_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'active'::text, 'suspended'::text, 'inactive'::text])))
);

CREATE TABLE IF NOT EXISTS public.submissions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_name text NOT NULL,
    category text,
    address text,
    city text,
    state text,
    zip text,
    phone text,
    email text,
    website text,
    description text,
    submitted_by text,
    submitter_email text,
    directory_id uuid,
    status text DEFAULT 'pending'::text,
    admin_notes text,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE OR REPLACE VIEW public.supplier_order_stats AS
 SELECT supplier_business_id,
    count(*) AS total_orders,
    count(*) FILTER (WHERE (status = 'pending'::text)) AS pending_orders,
    count(*) FILTER (WHERE (status = 'confirmed'::text)) AS confirmed_orders,
    count(*) FILTER (WHERE (status = 'shipped'::text)) AS shipped_orders,
    count(*) FILTER (WHERE (status = 'delivered'::text)) AS delivered_orders,
    count(*) FILTER (WHERE (status = 'cancelled'::text)) AS cancelled_orders,
    COALESCE(sum(total_amount) FILTER (WHERE (status <> 'cancelled'::text)), (0)::numeric) AS total_revenue,
    COALESCE(avg(buyer_rating), (0)::numeric) AS avg_rating
   FROM public.b2b_orders
  GROUP BY supplier_business_id;

CREATE TABLE IF NOT EXISTS public.supplier_products (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    business_id uuid NOT NULL,
    name text NOT NULL,
    description text,
    category text,
    price numeric(10,2),
    unit text DEFAULT 'each'::text,
    min_order integer DEFAULT 1,
    currency text DEFAULT 'USD'::text,
    delivery_areas text[] DEFAULT '{}'::text[],
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.survey_responses (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    survey_id uuid NOT NULL,
    visitor_account_id uuid,
    visitor_fingerprint text,
    directory_id uuid NOT NULL,
    answers jsonb DEFAULT '{}'::jsonb NOT NULL,
    applied_tags text[] DEFAULT '{}'::text[] NOT NULL,
    completed_at timestamp with time zone DEFAULT now() NOT NULL,
    audience text DEFAULT 'customer'::text NOT NULL,
    reward_units_awarded integer DEFAULT 0 NOT NULL,
    currency_name text,
    coreswift_pushed boolean DEFAULT false NOT NULL,
    coreswift_push_error text,
    coreswift_contact_id uuid
);

COMMENT ON COLUMN public.survey_responses.reward_units_awarded IS 'Native currency units actually credited for this response (0 when the respondent could not be identified or the programme awards nothing).';

COMMENT ON COLUMN public.survey_responses.coreswift_pushed IS 'True only when CoreSwift really accepted the answers. A skipped (CRM not connected) or failed push stays false, and the reason is in coreswift_push_error — never faked.';

CREATE TABLE IF NOT EXISTS public.tag_rules (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    tag_id uuid NOT NULL,
    trigger_type text NOT NULL,
    action_type text NOT NULL,
    action_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    CONSTRAINT tag_rules_action_type_check CHECK ((action_type = ANY (ARRAY['send_email'::text, 'send_sms'::text, 'webhook'::text, 'pipeline_move'::text, 'scoring_update'::text, 'add_tag'::text, 'remove_tag'::text, 'issue_voucher'::text]))),
    CONSTRAINT tag_rules_trigger_type_check CHECK ((trigger_type = ANY (ARRAY['tag_applied'::text, 'tag_removed'::text, 'workflow_completed'::text])))
);

CREATE TABLE IF NOT EXISTS public.template_categories (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    slug text NOT NULL,
    name text NOT NULL,
    description text,
    icon character varying(50) DEFAULT '📁'::character varying,
    sort_order integer DEFAULT 0 NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.tenants (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name character varying(255) NOT NULL,
    slug character varying(100) NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    industry_slug character varying(100) DEFAULT 'site-flipping'::character varying
);

CREATE TABLE IF NOT EXISTS public.topic_format_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    name text NOT NULL,
    template text NOT NULL,
    is_active boolean DEFAULT true,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.tracked_links (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    url text NOT NULL,
    utm_source text,
    utm_medium text,
    utm_campaign text,
    utm_content text,
    short_code text,
    is_active boolean DEFAULT true,
    total_clicks integer DEFAULT 0,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.trap_door_templates (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    directory_id uuid NOT NULL,
    name text NOT NULL,
    pattern text NOT NULL,
    placeholders jsonb DEFAULT '[]'::jsonb,
    is_active boolean DEFAULT true,
    last_generated_at timestamp with time zone,
    page_count integer DEFAULT 0,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.twilio_numbers (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    phone_number text NOT NULL,
    friendly_name text,
    sid text,
    provider text DEFAULT 'telnyx'::text,
    directory_id uuid,
    business_id uuid,
    forwarding_number text,
    webhook_url text,
    call_logging boolean DEFAULT true,
    monthly_cost numeric(8,2),
    status text DEFAULT 'active'::text,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.users (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid NOT NULL,
    email character varying(255) NOT NULL,
    password_hash text NOT NULL,
    name character varying(255) NOT NULL,
    role character varying(50) DEFAULT 'staff'::character varying NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    last_login_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT users_email_format_check CHECK (((email)::text ~ '^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$'::text))
);

CREATE TABLE IF NOT EXISTS public.visitor_accounts (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    email text NOT NULL,
    password_hash text NOT NULL,
    name text,
    phone text,
    directory_id uuid,
    is_active boolean DEFAULT true,
    last_login_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    interest_tags text[] DEFAULT '{}'::text[] NOT NULL,
    business_type text,
    survey_answered_at timestamp with time zone,
    coreswift_contact_id uuid,
    CONSTRAINT visitor_accounts_email_format_check CHECK ((email ~ '^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$'::text))
);

CREATE TABLE IF NOT EXISTS public.visitor_events (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    visitor_id uuid,
    session_id uuid,
    directory_id uuid,
    business_id uuid,
    event_type text NOT NULL,
    event_value text,
    metadata jsonb,
    page_url text,
    scroll_depth integer DEFAULT 0,
    duration_ms integer DEFAULT 0,
    created_at timestamp with time zone DEFAULT now(),
    category_id uuid
);

CREATE TABLE IF NOT EXISTS public.visitor_favorites (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    visitor_account_id uuid NOT NULL,
    business_id uuid NOT NULL,
    directory_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS public.visitor_sessions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    visitor_id uuid,
    directory_id uuid,
    referrer text,
    utm_source text,
    utm_medium text,
    utm_campaign text,
    utm_term text,
    utm_content text,
    landing_page text,
    exit_page text,
    pages_viewed integer DEFAULT 0,
    scroll_depth_pct integer DEFAULT 0,
    time_on_page_secs integer DEFAULT 0,
    is_bounce boolean DEFAULT true,
    duration_secs integer DEFAULT 0,
    entry_url text,
    exit_url text,
    started_at timestamp with time zone DEFAULT now(),
    ended_at timestamp with time zone
);

CREATE TABLE IF NOT EXISTS public.visitors (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    fingerprint text,
    user_agent text,
    ip_address text,
    language text,
    screen_resolution text,
    timezone text,
    city text,
    region text,
    country text,
    isp text,
    is_claimed_owner boolean DEFAULT false,
    first_seen_at timestamp with time zone DEFAULT now(),
    last_seen_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.webhook_deliveries (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    webhook_id uuid NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    attempt_count integer DEFAULT 0,
    max_attempts integer DEFAULT 3,
    response_status_code integer,
    response_body text,
    error_message text,
    next_retry_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.webhooks (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_id uuid,
    tenant_id uuid,
    directory_id uuid,
    url text NOT NULL,
    events text[] DEFAULT '{}'::text[] NOT NULL,
    secret text,
    is_active boolean DEFAULT true,
    retry_count integer DEFAULT 3,
    timeout_seconds integer DEFAULT 10,
    last_triggered_at timestamp with time zone,
    last_success_at timestamp with time zone,
    last_failure_at timestamp with time zone,
    failure_count integer DEFAULT 0,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.zaarhub_legal_pages (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid DEFAULT '00000000-0000-0000-0000-000000000001'::uuid NOT NULL,
    slug character varying(128) NOT NULL,
    title character varying(255) NOT NULL,
    content text NOT NULL,
    is_published boolean DEFAULT false,
    show_in_footer boolean DEFAULT false,
    display_order integer DEFAULT 0,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

CREATE TABLE IF NOT EXISTS public.zaarhub_site_config (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    tenant_id uuid DEFAULT '00000000-0000-0000-0000-000000000001'::uuid NOT NULL,
    site_name character varying(255) DEFAULT 'ZaarHub'::character varying,
    site_tagline text DEFAULT 'Discover Your Local Community'::text,
    primary_color character varying(7) DEFAULT '#f27f2f'::character varying,
    secondary_color character varying(7) DEFAULT '#2b3255'::character varying,
    logo_url text,
    favicon_url text,
    google_analytics_id character varying(64),
    facebook_app_id character varying(64),
    twitter_handle character varying(32),
    contact_email character varying(255),
    contact_phone character varying(32),
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    copyright_year character varying(32)
);

ALTER TABLE ONLY public.app_encryption_config ALTER COLUMN id SET DEFAULT nextval('public.app_encryption_config_id_seq'::regclass);

-- public._city_tags._city_tags_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = '_city_tags_pkey' AND conrelid = 'public._city_tags'::regclass
    ) THEN
        ALTER TABLE public._city_tags ADD CONSTRAINT _city_tags_pkey PRIMARY KEY (directory_id, tag_name);
    END IF;
END
$md_bl$;

-- public.account_links.account_links_email_user_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'account_links_email_user_id_key' AND conrelid = 'public.account_links'::regclass
    ) THEN
        ALTER TABLE public.account_links ADD CONSTRAINT account_links_email_user_id_key UNIQUE (email, user_id);
    END IF;
END
$md_bl$;

-- public.account_links.account_links_email_visitor_account_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'account_links_email_visitor_account_id_key' AND conrelid = 'public.account_links'::regclass
    ) THEN
        ALTER TABLE public.account_links ADD CONSTRAINT account_links_email_visitor_account_id_key UNIQUE (email, visitor_account_id);
    END IF;
END
$md_bl$;

-- public.account_links.account_links_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'account_links_pkey' AND conrelid = 'public.account_links'::regclass
    ) THEN
        ALTER TABLE public.account_links ADD CONSTRAINT account_links_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.ad_creatives.ad_creatives_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_creatives_pkey' AND conrelid = 'public.ad_creatives'::regclass
    ) THEN
        ALTER TABLE public.ad_creatives ADD CONSTRAINT ad_creatives_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.ad_earnings.ad_earnings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_earnings_pkey' AND conrelid = 'public.ad_earnings'::regclass
    ) THEN
        ALTER TABLE public.ad_earnings ADD CONSTRAINT ad_earnings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.ad_schedules.ad_schedules_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_schedules_pkey' AND conrelid = 'public.ad_schedules'::regclass
    ) THEN
        ALTER TABLE public.ad_schedules ADD CONSTRAINT ad_schedules_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.ad_zones.ad_zones_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_zones_pkey' AND conrelid = 'public.ad_zones'::regclass
    ) THEN
        ALTER TABLE public.ad_zones ADD CONSTRAINT ad_zones_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.analytics_events.analytics_events_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'analytics_events_pkey' AND conrelid = 'public.analytics_events'::regclass
    ) THEN
        ALTER TABLE public.analytics_events ADD CONSTRAINT analytics_events_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.api_key_usage.api_key_usage_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'api_key_usage_pkey' AND conrelid = 'public.api_key_usage'::regclass
    ) THEN
        ALTER TABLE public.api_key_usage ADD CONSTRAINT api_key_usage_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.api_keys.api_keys_key_hash_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'api_keys_key_hash_key' AND conrelid = 'public.api_keys'::regclass
    ) THEN
        ALTER TABLE public.api_keys ADD CONSTRAINT api_keys_key_hash_key UNIQUE (key_hash);
    END IF;
END
$md_bl$;

-- public.api_keys.api_keys_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'api_keys_pkey' AND conrelid = 'public.api_keys'::regclass
    ) THEN
        ALTER TABLE public.api_keys ADD CONSTRAINT api_keys_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.app_encryption_config.app_encryption_config_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'app_encryption_config_pkey' AND conrelid = 'public.app_encryption_config'::regclass
    ) THEN
        ALTER TABLE public.app_encryption_config ADD CONSTRAINT app_encryption_config_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.approval_queue.approval_queue_item_type_item_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'approval_queue_item_type_item_id_key' AND conrelid = 'public.approval_queue'::regclass
    ) THEN
        ALTER TABLE public.approval_queue ADD CONSTRAINT approval_queue_item_type_item_id_key UNIQUE (item_type, item_id);
    END IF;
END
$md_bl$;

-- public.approval_queue.approval_queue_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'approval_queue_pkey' AND conrelid = 'public.approval_queue'::regclass
    ) THEN
        ALTER TABLE public.approval_queue ADD CONSTRAINT approval_queue_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.author_profiles.author_profiles_directory_id_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'author_profiles_directory_id_slug_key' AND conrelid = 'public.author_profiles'::regclass
    ) THEN
        ALTER TABLE public.author_profiles ADD CONSTRAINT author_profiles_directory_id_slug_key UNIQUE (directory_id, slug);
    END IF;
END
$md_bl$;

-- public.author_profiles.author_profiles_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'author_profiles_pkey' AND conrelid = 'public.author_profiles'::regclass
    ) THEN
        ALTER TABLE public.author_profiles ADD CONSTRAINT author_profiles_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.author_profiles.author_profiles_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'author_profiles_slug_key' AND conrelid = 'public.author_profiles'::regclass
    ) THEN
        ALTER TABLE public.author_profiles ADD CONSTRAINT author_profiles_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.available_providers.available_providers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'available_providers_pkey' AND conrelid = 'public.available_providers'::regclass
    ) THEN
        ALTER TABLE public.available_providers ADD CONSTRAINT available_providers_pkey PRIMARY KEY (key);
    END IF;
END
$md_bl$;

-- public.b2b_notifications.b2b_notifications_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_notifications_pkey' AND conrelid = 'public.b2b_notifications'::regclass
    ) THEN
        ALTER TABLE public.b2b_notifications ADD CONSTRAINT b2b_notifications_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.b2b_orders.b2b_orders_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_orders_pkey' AND conrelid = 'public.b2b_orders'::regclass
    ) THEN
        ALTER TABLE public.b2b_orders ADD CONSTRAINT b2b_orders_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.blog_media.blog_media_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_media_pkey' AND conrelid = 'public.blog_media'::regclass
    ) THEN
        ALTER TABLE public.blog_media ADD CONSTRAINT blog_media_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_pkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.blog_qa_keywords.blog_qa_keywords_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_qa_keywords_pkey' AND conrelid = 'public.blog_qa_keywords'::regclass
    ) THEN
        ALTER TABLE public.blog_qa_keywords ADD CONSTRAINT blog_qa_keywords_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.blog_qa_posts.blog_qa_posts_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_qa_posts_pkey' AND conrelid = 'public.blog_qa_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_qa_posts ADD CONSTRAINT blog_qa_posts_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.blog_template_directories.blog_template_directories_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_template_directories_pkey' AND conrelid = 'public.blog_template_directories'::regclass
    ) THEN
        ALTER TABLE public.blog_template_directories ADD CONSTRAINT blog_template_directories_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.blog_template_directories.blog_template_directories_template_id_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_template_directories_template_id_directory_id_key' AND conrelid = 'public.blog_template_directories'::regclass
    ) THEN
        ALTER TABLE public.blog_template_directories ADD CONSTRAINT blog_template_directories_template_id_directory_id_key UNIQUE (template_id, directory_id);
    END IF;
END
$md_bl$;

-- public.blog_templates.blog_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_templates_pkey' AND conrelid = 'public.blog_templates'::regclass
    ) THEN
        ALTER TABLE public.blog_templates ADD CONSTRAINT blog_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.bundle_services.bundle_services_bundle_id_service_key_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'bundle_services_bundle_id_service_key_key' AND conrelid = 'public.bundle_services'::regclass
    ) THEN
        ALTER TABLE public.bundle_services ADD CONSTRAINT bundle_services_bundle_id_service_key_key UNIQUE (bundle_id, service_key);
    END IF;
END
$md_bl$;

-- public.bundle_services.bundle_services_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'bundle_services_pkey' AND conrelid = 'public.bundle_services'::regclass
    ) THEN
        ALTER TABLE public.bundle_services ADD CONSTRAINT bundle_services_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_articles.business_articles_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_articles_pkey' AND conrelid = 'public.business_articles'::regclass
    ) THEN
        ALTER TABLE public.business_articles ADD CONSTRAINT business_articles_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_categories.business_categories_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_categories_pkey' AND conrelid = 'public.business_categories'::regclass
    ) THEN
        ALTER TABLE public.business_categories ADD CONSTRAINT business_categories_pkey PRIMARY KEY (business_id, category_id);
    END IF;
END
$md_bl$;

-- public.business_listings.business_listings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_listings_pkey' AND conrelid = 'public.business_listings'::regclass
    ) THEN
        ALTER TABLE public.business_listings ADD CONSTRAINT business_listings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_messages.business_messages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_messages_pkey' AND conrelid = 'public.business_messages'::regclass
    ) THEN
        ALTER TABLE public.business_messages ADD CONSTRAINT business_messages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_meta.business_meta_business_id_template_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_meta_business_id_template_key' AND conrelid = 'public.business_meta'::regclass
    ) THEN
        ALTER TABLE public.business_meta ADD CONSTRAINT business_meta_business_id_template_key UNIQUE (business_id, template);
    END IF;
END
$md_bl$;

-- public.business_meta.business_meta_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_meta_pkey' AND conrelid = 'public.business_meta'::regclass
    ) THEN
        ALTER TABLE public.business_meta ADD CONSTRAINT business_meta_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_point_ledger.business_point_ledger_network_id_business_id_month_key_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_point_ledger_network_id_business_id_month_key_key' AND conrelid = 'public.business_point_ledger'::regclass
    ) THEN
        ALTER TABLE public.business_point_ledger ADD CONSTRAINT business_point_ledger_network_id_business_id_month_key_key UNIQUE (network_id, business_id, month_key);
    END IF;
END
$md_bl$;

-- public.business_point_ledger.business_point_ledger_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_point_ledger_pkey' AND conrelid = 'public.business_point_ledger'::regclass
    ) THEN
        ALTER TABLE public.business_point_ledger ADD CONSTRAINT business_point_ledger_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_services.business_services_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_services_pkey' AND conrelid = 'public.business_services'::regclass
    ) THEN
        ALTER TABLE public.business_services ADD CONSTRAINT business_services_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_subscriptions.business_subscriptions_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_subscriptions_pkey' AND conrelid = 'public.business_subscriptions'::regclass
    ) THEN
        ALTER TABLE public.business_subscriptions ADD CONSTRAINT business_subscriptions_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_transfer_events.business_transfer_events_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfer_events_pkey' AND conrelid = 'public.business_transfer_events'::regclass
    ) THEN
        ALTER TABLE public.business_transfer_events ADD CONSTRAINT business_transfer_events_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_transfer_fees.business_transfer_fees_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfer_fees_pkey' AND conrelid = 'public.business_transfer_fees'::regclass
    ) THEN
        ALTER TABLE public.business_transfer_fees ADD CONSTRAINT business_transfer_fees_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_transfers.business_transfers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfers_pkey' AND conrelid = 'public.business_transfers'::regclass
    ) THEN
        ALTER TABLE public.business_transfers ADD CONSTRAINT business_transfers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.business_verifications.business_verifications_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_verifications_pkey' AND conrelid = 'public.business_verifications'::regclass
    ) THEN
        ALTER TABLE public.business_verifications ADD CONSTRAINT business_verifications_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.businesses.businesses_directory_id_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'businesses_directory_id_slug_key' AND conrelid = 'public.businesses'::regclass
    ) THEN
        ALTER TABLE public.businesses ADD CONSTRAINT businesses_directory_id_slug_key UNIQUE (directory_id, slug);
    END IF;
END
$md_bl$;

-- public.businesses.businesses_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'businesses_pkey' AND conrelid = 'public.businesses'::regclass
    ) THEN
        ALTER TABLE public.businesses ADD CONSTRAINT businesses_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.buying_group_deals.buying_group_deals_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_deals_pkey' AND conrelid = 'public.buying_group_deals'::regclass
    ) THEN
        ALTER TABLE public.buying_group_deals ADD CONSTRAINT buying_group_deals_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.buying_group_members.buying_group_members_group_id_business_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_members_group_id_business_id_key' AND conrelid = 'public.buying_group_members'::regclass
    ) THEN
        ALTER TABLE public.buying_group_members ADD CONSTRAINT buying_group_members_group_id_business_id_key UNIQUE (group_id, business_id);
    END IF;
END
$md_bl$;

-- public.buying_group_members.buying_group_members_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_members_pkey' AND conrelid = 'public.buying_group_members'::regclass
    ) THEN
        ALTER TABLE public.buying_group_members ADD CONSTRAINT buying_group_members_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.buying_groups.buying_groups_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_groups_pkey' AND conrelid = 'public.buying_groups'::regclass
    ) THEN
        ALTER TABLE public.buying_groups ADD CONSTRAINT buying_groups_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.call_logs.call_logs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'call_logs_pkey' AND conrelid = 'public.call_logs'::regclass
    ) THEN
        ALTER TABLE public.call_logs ADD CONSTRAINT call_logs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.category_redeem_caps.category_redeem_caps_network_id_category_name_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'category_redeem_caps_network_id_category_name_key' AND conrelid = 'public.category_redeem_caps'::regclass
    ) THEN
        ALTER TABLE public.category_redeem_caps ADD CONSTRAINT category_redeem_caps_network_id_category_name_key UNIQUE (network_id, category_name);
    END IF;
END
$md_bl$;

-- public.category_redeem_caps.category_redeem_caps_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'category_redeem_caps_pkey' AND conrelid = 'public.category_redeem_caps'::regclass
    ) THEN
        ALTER TABLE public.category_redeem_caps ADD CONSTRAINT category_redeem_caps_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.category_requests.category_requests_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'category_requests_pkey' AND conrelid = 'public.category_requests'::regclass
    ) THEN
        ALTER TABLE public.category_requests ADD CONSTRAINT category_requests_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.checkout_sessions.checkout_sessions_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'checkout_sessions_pkey' AND conrelid = 'public.checkout_sessions'::regclass
    ) THEN
        ALTER TABLE public.checkout_sessions ADD CONSTRAINT checkout_sessions_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.city_pages.city_pages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_pages_pkey' AND conrelid = 'public.city_pages'::regclass
    ) THEN
        ALTER TABLE public.city_pages ADD CONSTRAINT city_pages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.city_plan_slots.city_plan_slots_city_slug_plan_tier_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_plan_slots_city_slug_plan_tier_id_key' AND conrelid = 'public.city_plan_slots'::regclass
    ) THEN
        ALTER TABLE public.city_plan_slots ADD CONSTRAINT city_plan_slots_city_slug_plan_tier_id_key UNIQUE (city_slug, plan_tier_id);
    END IF;
END
$md_bl$;

-- public.city_plan_slots.city_plan_slots_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_plan_slots_pkey' AND conrelid = 'public.city_plan_slots'::regclass
    ) THEN
        ALTER TABLE public.city_plan_slots ADD CONSTRAINT city_plan_slots_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.city_priority.city_priority_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_priority_pkey' AND conrelid = 'public.city_priority'::regclass
    ) THEN
        ALTER TABLE public.city_priority ADD CONSTRAINT city_priority_pkey PRIMARY KEY (directory_slug, city_name);
    END IF;
END
$md_bl$;

-- public.city_requests.city_requests_city_state_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_requests_city_state_key' AND conrelid = 'public.city_requests'::regclass
    ) THEN
        ALTER TABLE public.city_requests ADD CONSTRAINT city_requests_city_state_key UNIQUE (city_name, state);
    END IF;
END
$md_bl$;

-- public.city_requests.city_requests_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_requests_pkey' AND conrelid = 'public.city_requests'::regclass
    ) THEN
        ALTER TABLE public.city_requests ADD CONSTRAINT city_requests_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.claim_offers.claim_offers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claim_offers_pkey' AND conrelid = 'public.claim_offers'::regclass
    ) THEN
        ALTER TABLE public.claim_offers ADD CONSTRAINT claim_offers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.claimed_businesses.claimed_businesses_business_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claimed_businesses_business_id_key' AND conrelid = 'public.claimed_businesses'::regclass
    ) THEN
        ALTER TABLE public.claimed_businesses ADD CONSTRAINT claimed_businesses_business_id_key UNIQUE (business_id);
    END IF;
END
$md_bl$;

-- public.claimed_businesses.claimed_businesses_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claimed_businesses_pkey' AND conrelid = 'public.claimed_businesses'::regclass
    ) THEN
        ALTER TABLE public.claimed_businesses ADD CONSTRAINT claimed_businesses_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.community_events.community_events_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'community_events_pkey' AND conrelid = 'public.community_events'::regclass
    ) THEN
        ALTER TABLE public.community_events ADD CONSTRAINT community_events_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.connected_services.connected_services_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'connected_services_pkey' AND conrelid = 'public.connected_services'::regclass
    ) THEN
        ALTER TABLE public.connected_services ADD CONSTRAINT connected_services_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.connected_services.connected_services_user_id_service_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'connected_services_user_id_service_key' AND conrelid = 'public.connected_services'::regclass
    ) THEN
        ALTER TABLE public.connected_services ADD CONSTRAINT connected_services_user_id_service_key UNIQUE (user_id, service);
    END IF;
END
$md_bl$;

-- public.content_queue.content_queue_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_queue_pkey' AND conrelid = 'public.content_queue'::regclass
    ) THEN
        ALTER TABLE public.content_queue ADD CONSTRAINT content_queue_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.content_research.content_research_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_research_pkey' AND conrelid = 'public.content_research'::regclass
    ) THEN
        ALTER TABLE public.content_research ADD CONSTRAINT content_research_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.content_topics.content_topics_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_topics_pkey' AND conrelid = 'public.content_topics'::regclass
    ) THEN
        ALTER TABLE public.content_topics ADD CONSTRAINT content_topics_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.crm_contacts.crm_contacts_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_contacts_pkey' AND conrelid = 'public.crm_contacts'::regclass
    ) THEN
        ALTER TABLE public.crm_contacts ADD CONSTRAINT crm_contacts_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.crm_deal_records.crm_deal_records_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_deal_records_pkey' AND conrelid = 'public.crm_deal_records'::regclass
    ) THEN
        ALTER TABLE public.crm_deal_records ADD CONSTRAINT crm_deal_records_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.crm_pipelines.crm_pipelines_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_pipelines_pkey' AND conrelid = 'public.crm_pipelines'::regclass
    ) THEN
        ALTER TABLE public.crm_pipelines ADD CONSTRAINT crm_pipelines_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.data_enrichment_logs.data_enrichment_logs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'data_enrichment_logs_pkey' AND conrelid = 'public.data_enrichment_logs'::regclass
    ) THEN
        ALTER TABLE public.data_enrichment_logs ADD CONSTRAINT data_enrichment_logs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.deal_claims.deal_claims_claim_code_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_claims_claim_code_key' AND conrelid = 'public.deal_claims'::regclass
    ) THEN
        ALTER TABLE public.deal_claims ADD CONSTRAINT deal_claims_claim_code_key UNIQUE (claim_code);
    END IF;
END
$md_bl$;

-- public.deal_claims.deal_claims_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_claims_pkey' AND conrelid = 'public.deal_claims'::regclass
    ) THEN
        ALTER TABLE public.deal_claims ADD CONSTRAINT deal_claims_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.deal_redemptions.deal_redemptions_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_redemptions_pkey' AND conrelid = 'public.deal_redemptions'::regclass
    ) THEN
        ALTER TABLE public.deal_redemptions ADD CONSTRAINT deal_redemptions_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.deal_redemptions.deal_redemptions_redemption_code_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_redemptions_redemption_code_key' AND conrelid = 'public.deal_redemptions'::regclass
    ) THEN
        ALTER TABLE public.deal_redemptions ADD CONSTRAINT deal_redemptions_redemption_code_key UNIQUE (redemption_code);
    END IF;
END
$md_bl$;

-- public.deal_templates.deal_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_templates_pkey' AND conrelid = 'public.deal_templates'::regclass
    ) THEN
        ALTER TABLE public.deal_templates ADD CONSTRAINT deal_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.deals.deals_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deals_pkey' AND conrelid = 'public.deals'::regclass
    ) THEN
        ALTER TABLE public.deals ADD CONSTRAINT deals_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.demand_analytics_settings.demand_analytics_settings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'demand_analytics_settings_pkey' AND conrelid = 'public.demand_analytics_settings'::regclass
    ) THEN
        ALTER TABLE public.demand_analytics_settings ADD CONSTRAINT demand_analytics_settings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directories.directories_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directories_pkey' AND conrelid = 'public.directories'::regclass
    ) THEN
        ALTER TABLE public.directories ADD CONSTRAINT directories_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directories.directories_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directories_slug_key' AND conrelid = 'public.directories'::regclass
    ) THEN
        ALTER TABLE public.directories ADD CONSTRAINT directories_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.directory_branding.directory_branding_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_branding_directory_id_key' AND conrelid = 'public.directory_branding'::regclass
    ) THEN
        ALTER TABLE public.directory_branding ADD CONSTRAINT directory_branding_directory_id_key UNIQUE (directory_id);
    END IF;
END
$md_bl$;

-- public.directory_branding.directory_branding_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_branding_pkey' AND conrelid = 'public.directory_branding'::regclass
    ) THEN
        ALTER TABLE public.directory_branding ADD CONSTRAINT directory_branding_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_categories.directory_categories_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_categories_pkey' AND conrelid = 'public.directory_categories'::regclass
    ) THEN
        ALTER TABLE public.directory_categories ADD CONSTRAINT directory_categories_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_categories.directory_categories_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_categories_slug_key' AND conrelid = 'public.directory_categories'::regclass
    ) THEN
        ALTER TABLE public.directory_categories ADD CONSTRAINT directory_categories_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.directory_email_settings.directory_email_settings_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_email_settings_directory_id_key' AND conrelid = 'public.directory_email_settings'::regclass
    ) THEN
        ALTER TABLE public.directory_email_settings ADD CONSTRAINT directory_email_settings_directory_id_key UNIQUE (directory_id);
    END IF;
END
$md_bl$;

-- public.directory_email_settings.directory_email_settings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_email_settings_pkey' AND conrelid = 'public.directory_email_settings'::regclass
    ) THEN
        ALTER TABLE public.directory_email_settings ADD CONSTRAINT directory_email_settings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_events.directory_events_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_events_pkey' AND conrelid = 'public.directory_events'::regclass
    ) THEN
        ALTER TABLE public.directory_events ADD CONSTRAINT directory_events_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_locations.directory_locations_directory_id_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_locations_directory_id_slug_key' AND conrelid = 'public.directory_locations'::regclass
    ) THEN
        ALTER TABLE public.directory_locations ADD CONSTRAINT directory_locations_directory_id_slug_key UNIQUE (directory_id, slug);
    END IF;
END
$md_bl$;

-- public.directory_locations.directory_locations_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_locations_pkey' AND conrelid = 'public.directory_locations'::regclass
    ) THEN
        ALTER TABLE public.directory_locations ADD CONSTRAINT directory_locations_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_notifications.directory_notifications_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_notifications_pkey' AND conrelid = 'public.directory_notifications'::regclass
    ) THEN
        ALTER TABLE public.directory_notifications ADD CONSTRAINT directory_notifications_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_services.directory_services_directory_id_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_services_directory_id_slug_key' AND conrelid = 'public.directory_services'::regclass
    ) THEN
        ALTER TABLE public.directory_services ADD CONSTRAINT directory_services_directory_id_slug_key UNIQUE (directory_id, slug);
    END IF;
END
$md_bl$;

-- public.directory_services.directory_services_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_services_pkey' AND conrelid = 'public.directory_services'::regclass
    ) THEN
        ALTER TABLE public.directory_services ADD CONSTRAINT directory_services_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_surveys.directory_surveys_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_surveys_pkey' AND conrelid = 'public.directory_surveys'::regclass
    ) THEN
        ALTER TABLE public.directory_surveys ADD CONSTRAINT directory_surveys_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.directory_tiers.directory_tiers_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_tiers_directory_id_key' AND conrelid = 'public.directory_tiers'::regclass
    ) THEN
        ALTER TABLE public.directory_tiers ADD CONSTRAINT directory_tiers_directory_id_key UNIQUE (directory_id);
    END IF;
END
$md_bl$;

-- public.directory_tiers.directory_tiers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_tiers_pkey' AND conrelid = 'public.directory_tiers'::regclass
    ) THEN
        ALTER TABLE public.directory_tiers ADD CONSTRAINT directory_tiers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.discovery_queue.discovery_queue_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'discovery_queue_pkey' AND conrelid = 'public.discovery_queue'::regclass
    ) THEN
        ALTER TABLE public.discovery_queue ADD CONSTRAINT discovery_queue_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.domain_mappings.domain_mappings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'domain_mappings_pkey' AND conrelid = 'public.domain_mappings'::regclass
    ) THEN
        ALTER TABLE public.domain_mappings ADD CONSTRAINT domain_mappings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.email_campaigns.email_campaigns_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'email_campaigns_pkey' AND conrelid = 'public.email_campaigns'::regclass
    ) THEN
        ALTER TABLE public.email_campaigns ADD CONSTRAINT email_campaigns_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.email_templates.email_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'email_templates_pkey' AND conrelid = 'public.email_templates'::regclass
    ) THEN
        ALTER TABLE public.email_templates ADD CONSTRAINT email_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.enrichment_settings.enrichment_settings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'enrichment_settings_pkey' AND conrelid = 'public.enrichment_settings'::regclass
    ) THEN
        ALTER TABLE public.enrichment_settings ADD CONSTRAINT enrichment_settings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.event_providers.event_providers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'event_providers_pkey' AND conrelid = 'public.event_providers'::regclass
    ) THEN
        ALTER TABLE public.event_providers ADD CONSTRAINT event_providers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.event_rsvps.event_rsvps_event_id_visitor_account_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'event_rsvps_event_id_visitor_account_id_key' AND conrelid = 'public.event_rsvps'::regclass
    ) THEN
        ALTER TABLE public.event_rsvps ADD CONSTRAINT event_rsvps_event_id_visitor_account_id_key UNIQUE (event_id, visitor_account_id);
    END IF;
END
$md_bl$;

-- public.event_rsvps.event_rsvps_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'event_rsvps_pkey' AND conrelid = 'public.event_rsvps'::regclass
    ) THEN
        ALTER TABLE public.event_rsvps ADD CONSTRAINT event_rsvps_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.export_templates.export_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'export_templates_pkey' AND conrelid = 'public.export_templates'::regclass
    ) THEN
        ALTER TABLE public.export_templates ADD CONSTRAINT export_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.google_places_cache.google_places_cache_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'google_places_cache_pkey' AND conrelid = 'public.google_places_cache'::regclass
    ) THEN
        ALTER TABLE public.google_places_cache ADD CONSTRAINT google_places_cache_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.grandfathered_pricing.grandfathered_pricing_business_id_service_key_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'grandfathered_pricing_business_id_service_key_key' AND conrelid = 'public.grandfathered_pricing'::regclass
    ) THEN
        ALTER TABLE public.grandfathered_pricing ADD CONSTRAINT grandfathered_pricing_business_id_service_key_key UNIQUE (business_id, service_key);
    END IF;
END
$md_bl$;

-- public.grandfathered_pricing.grandfathered_pricing_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'grandfathered_pricing_pkey' AND conrelid = 'public.grandfathered_pricing'::regclass
    ) THEN
        ALTER TABLE public.grandfathered_pricing ADD CONSTRAINT grandfathered_pricing_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.group_deal_commitments.group_deal_commitments_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'group_deal_commitments_pkey' AND conrelid = 'public.group_deal_commitments'::regclass
    ) THEN
        ALTER TABLE public.group_deal_commitments ADD CONSTRAINT group_deal_commitments_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.homepage_sections.homepage_sections_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_sections_pkey' AND conrelid = 'public.homepage_sections'::regclass
    ) THEN
        ALTER TABLE public.homepage_sections ADD CONSTRAINT homepage_sections_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.import_logs.import_logs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'import_logs_pkey' AND conrelid = 'public.import_logs'::regclass
    ) THEN
        ALTER TABLE public.import_logs ADD CONSTRAINT import_logs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.integration_configs.integration_configs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'integration_configs_pkey' AND conrelid = 'public.integration_configs'::regclass
    ) THEN
        ALTER TABLE public.integration_configs ADD CONSTRAINT integration_configs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.integration_configs.integration_configs_provider_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'integration_configs_provider_key' AND conrelid = 'public.integration_configs'::regclass
    ) THEN
        ALTER TABLE public.integration_configs ADD CONSTRAINT integration_configs_provider_key UNIQUE (provider);
    END IF;
END
$md_bl$;

-- public.integration_provider_presets.integration_provider_presets_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'integration_provider_presets_pkey' AND conrelid = 'public.integration_provider_presets'::regclass
    ) THEN
        ALTER TABLE public.integration_provider_presets ADD CONSTRAINT integration_provider_presets_pkey PRIMARY KEY (key);
    END IF;
END
$md_bl$;

-- public.landing_pages.landing_pages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'landing_pages_pkey' AND conrelid = 'public.landing_pages'::regclass
    ) THEN
        ALTER TABLE public.landing_pages ADD CONSTRAINT landing_pages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.landing_pages.landing_pages_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'landing_pages_slug_key' AND conrelid = 'public.landing_pages'::regclass
    ) THEN
        ALTER TABLE public.landing_pages ADD CONSTRAINT landing_pages_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.lead_share_transactions.lead_share_transactions_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'lead_share_transactions_pkey' AND conrelid = 'public.lead_share_transactions'::regclass
    ) THEN
        ALTER TABLE public.lead_share_transactions ADD CONSTRAINT lead_share_transactions_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.legal_pages.legal_pages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'legal_pages_pkey' AND conrelid = 'public.legal_pages'::regclass
    ) THEN
        ALTER TABLE public.legal_pages ADD CONSTRAINT legal_pages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.link_clicks.link_clicks_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'link_clicks_pkey' AND conrelid = 'public.link_clicks'::regclass
    ) THEN
        ALTER TABLE public.link_clicks ADD CONSTRAINT link_clicks_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_activity.loyalty_activity_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_activity_pkey' AND conrelid = 'public.loyalty_activity'::regclass
    ) THEN
        ALTER TABLE public.loyalty_activity ADD CONSTRAINT loyalty_activity_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_checkins.loyalty_checkins_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_checkins_pkey' AND conrelid = 'public.loyalty_checkins'::regclass
    ) THEN
        ALTER TABLE public.loyalty_checkins ADD CONSTRAINT loyalty_checkins_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_enrollments.loyalty_enrollments_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_enrollments_pkey' AND conrelid = 'public.loyalty_enrollments'::regclass
    ) THEN
        ALTER TABLE public.loyalty_enrollments ADD CONSTRAINT loyalty_enrollments_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_enrollments.loyalty_enrollments_program_id_entity_type_entity_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_enrollments_program_id_entity_type_entity_id_key' AND conrelid = 'public.loyalty_enrollments'::regclass
    ) THEN
        ALTER TABLE public.loyalty_enrollments ADD CONSTRAINT loyalty_enrollments_program_id_entity_type_entity_id_key UNIQUE (program_id, entity_type, entity_id);
    END IF;
END
$md_bl$;

-- public.loyalty_members.loyalty_members_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_members_pkey' AND conrelid = 'public.loyalty_members'::regclass
    ) THEN
        ALTER TABLE public.loyalty_members ADD CONSTRAINT loyalty_members_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_members.loyalty_members_program_id_visitor_account_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_members_program_id_visitor_account_id_key' AND conrelid = 'public.loyalty_members'::regclass
    ) THEN
        ALTER TABLE public.loyalty_members ADD CONSTRAINT loyalty_members_program_id_visitor_account_id_key UNIQUE (program_id, visitor_account_id);
    END IF;
END
$md_bl$;

-- public.loyalty_members.loyalty_members_referral_code_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_members_referral_code_key' AND conrelid = 'public.loyalty_members'::regclass
    ) THEN
        ALTER TABLE public.loyalty_members ADD CONSTRAINT loyalty_members_referral_code_key UNIQUE (referral_code);
    END IF;
END
$md_bl$;

-- public.loyalty_milestones_completed.loyalty_milestones_completed_member_id_milestone_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_milestones_completed_member_id_milestone_id_key' AND conrelid = 'public.loyalty_milestones_completed'::regclass
    ) THEN
        ALTER TABLE public.loyalty_milestones_completed ADD CONSTRAINT loyalty_milestones_completed_member_id_milestone_id_key UNIQUE (member_id, milestone_id);
    END IF;
END
$md_bl$;

-- public.loyalty_milestones_completed.loyalty_milestones_completed_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_milestones_completed_pkey' AND conrelid = 'public.loyalty_milestones_completed'::regclass
    ) THEN
        ALTER TABLE public.loyalty_milestones_completed ADD CONSTRAINT loyalty_milestones_completed_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_milestones.loyalty_milestones_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_milestones_pkey' AND conrelid = 'public.loyalty_milestones'::regclass
    ) THEN
        ALTER TABLE public.loyalty_milestones ADD CONSTRAINT loyalty_milestones_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_programs.loyalty_programs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_programs_pkey' AND conrelid = 'public.loyalty_programs'::regclass
    ) THEN
        ALTER TABLE public.loyalty_programs ADD CONSTRAINT loyalty_programs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_reward_tiers.loyalty_reward_tiers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_reward_tiers_pkey' AND conrelid = 'public.loyalty_reward_tiers'::regclass
    ) THEN
        ALTER TABLE public.loyalty_reward_tiers ADD CONSTRAINT loyalty_reward_tiers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_rewards_earned.loyalty_rewards_earned_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_rewards_earned_pkey' AND conrelid = 'public.loyalty_rewards_earned'::regclass
    ) THEN
        ALTER TABLE public.loyalty_rewards_earned ADD CONSTRAINT loyalty_rewards_earned_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_scans.loyalty_scans_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_scans_pkey' AND conrelid = 'public.loyalty_scans'::regclass
    ) THEN
        ALTER TABLE public.loyalty_scans ADD CONSTRAINT loyalty_scans_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.loyalty_tiers.loyalty_tiers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_tiers_pkey' AND conrelid = 'public.loyalty_tiers'::regclass
    ) THEN
        ALTER TABLE public.loyalty_tiers ADD CONSTRAINT loyalty_tiers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.network_branding.network_branding_network_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'network_branding_network_id_key' AND conrelid = 'public.network_branding'::regclass
    ) THEN
        ALTER TABLE public.network_branding ADD CONSTRAINT network_branding_network_id_key UNIQUE (network_id);
    END IF;
END
$md_bl$;

-- public.network_branding.network_branding_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'network_branding_pkey' AND conrelid = 'public.network_branding'::regclass
    ) THEN
        ALTER TABLE public.network_branding ADD CONSTRAINT network_branding_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.networks.networks_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'networks_pkey' AND conrelid = 'public.networks'::regclass
    ) THEN
        ALTER TABLE public.networks ADD CONSTRAINT networks_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.networks.networks_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'networks_slug_key' AND conrelid = 'public.networks'::regclass
    ) THEN
        ALTER TABLE public.networks ADD CONSTRAINT networks_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.newsletter_digests.newsletter_digests_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_digests_pkey' AND conrelid = 'public.newsletter_digests'::regclass
    ) THEN
        ALTER TABLE public.newsletter_digests ADD CONSTRAINT newsletter_digests_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.newsletter_queue.newsletter_queue_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_queue_pkey' AND conrelid = 'public.newsletter_queue'::regclass
    ) THEN
        ALTER TABLE public.newsletter_queue ADD CONSTRAINT newsletter_queue_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.newsletter_subscribers.newsletter_subscribers_directory_id_email_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_subscribers_directory_id_email_key' AND conrelid = 'public.newsletter_subscribers'::regclass
    ) THEN
        ALTER TABLE public.newsletter_subscribers ADD CONSTRAINT newsletter_subscribers_directory_id_email_key UNIQUE (directory_id, email);
    END IF;
END
$md_bl$;

-- public.newsletter_subscribers.newsletter_subscribers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_subscribers_pkey' AND conrelid = 'public.newsletter_subscribers'::regclass
    ) THEN
        ALTER TABLE public.newsletter_subscribers ADD CONSTRAINT newsletter_subscribers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.offer_claims.offer_claims_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'offer_claims_pkey' AND conrelid = 'public.offer_claims'::regclass
    ) THEN
        ALTER TABLE public.offer_claims ADD CONSTRAINT offer_claims_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.password_resets.password_resets_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'password_resets_pkey' AND conrelid = 'public.password_resets'::regclass
    ) THEN
        ALTER TABLE public.password_resets ADD CONSTRAINT password_resets_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.password_resets.password_resets_token_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'password_resets_token_key' AND conrelid = 'public.password_resets'::regclass
    ) THEN
        ALTER TABLE public.password_resets ADD CONSTRAINT password_resets_token_key UNIQUE (token);
    END IF;
END
$md_bl$;

-- public.pay_per_call.pay_per_call_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'pay_per_call_pkey' AND conrelid = 'public.pay_per_call'::regclass
    ) THEN
        ALTER TABLE public.pay_per_call ADD CONSTRAINT pay_per_call_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.payment_providers.payment_providers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'payment_providers_pkey' AND conrelid = 'public.payment_providers'::regclass
    ) THEN
        ALTER TABLE public.payment_providers ADD CONSTRAINT payment_providers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.payment_providers.payment_providers_provider_type_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'payment_providers_provider_type_key' AND conrelid = 'public.payment_providers'::regclass
    ) THEN
        ALTER TABLE public.payment_providers ADD CONSTRAINT payment_providers_provider_type_key UNIQUE (provider_type);
    END IF;
END
$md_bl$;

-- public.payment_webhook_events.payment_webhook_events_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'payment_webhook_events_pkey' AND conrelid = 'public.payment_webhook_events'::regclass
    ) THEN
        ALTER TABLE public.payment_webhook_events ADD CONSTRAINT payment_webhook_events_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.plan_slot_bookings.plan_slot_bookings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'plan_slot_bookings_pkey' AND conrelid = 'public.plan_slot_bookings'::regclass
    ) THEN
        ALTER TABLE public.plan_slot_bookings ADD CONSTRAINT plan_slot_bookings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.plan_tiers.plan_tiers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'plan_tiers_pkey' AND conrelid = 'public.plan_tiers'::regclass
    ) THEN
        ALTER TABLE public.plan_tiers ADD CONSTRAINT plan_tiers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.plan_tiers.plan_tiers_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'plan_tiers_slug_key' AND conrelid = 'public.plan_tiers'::regclass
    ) THEN
        ALTER TABLE public.plan_tiers ADD CONSTRAINT plan_tiers_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.point_issuance_log.point_issuance_log_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_issuance_log_pkey' AND conrelid = 'public.point_issuance_log'::regclass
    ) THEN
        ALTER TABLE public.point_issuance_log ADD CONSTRAINT point_issuance_log_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.point_redemption_log.point_redemption_log_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_redemption_log_pkey' AND conrelid = 'public.point_redemption_log'::regclass
    ) THEN
        ALTER TABLE public.point_redemption_log ADD CONSTRAINT point_redemption_log_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.point_treasury.point_treasury_network_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_treasury_network_id_key' AND conrelid = 'public.point_treasury'::regclass
    ) THEN
        ALTER TABLE public.point_treasury ADD CONSTRAINT point_treasury_network_id_key UNIQUE (network_id);
    END IF;
END
$md_bl$;

-- public.point_treasury.point_treasury_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_treasury_pkey' AND conrelid = 'public.point_treasury'::regclass
    ) THEN
        ALTER TABLE public.point_treasury ADD CONSTRAINT point_treasury_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.poll_votes.poll_votes_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'poll_votes_pkey' AND conrelid = 'public.poll_votes'::regclass
    ) THEN
        ALTER TABLE public.poll_votes ADD CONSTRAINT poll_votes_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.poll_votes.poll_votes_poll_id_visitor_account_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'poll_votes_poll_id_visitor_account_id_key' AND conrelid = 'public.poll_votes'::regclass
    ) THEN
        ALTER TABLE public.poll_votes ADD CONSTRAINT poll_votes_poll_id_visitor_account_id_key UNIQUE (poll_id, visitor_account_id);
    END IF;
END
$md_bl$;

-- public.polls.polls_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'polls_pkey' AND conrelid = 'public.polls'::regclass
    ) THEN
        ALTER TABLE public.polls ADD CONSTRAINT polls_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.price_bundles.price_bundles_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'price_bundles_pkey' AND conrelid = 'public.price_bundles'::regclass
    ) THEN
        ALTER TABLE public.price_bundles ADD CONSTRAINT price_bundles_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.price_bundles.price_bundles_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'price_bundles_slug_key' AND conrelid = 'public.price_bundles'::regclass
    ) THEN
        ALTER TABLE public.price_bundles ADD CONSTRAINT price_bundles_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.programmatic_pages.programmatic_pages_directory_id_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'programmatic_pages_directory_id_slug_key' AND conrelid = 'public.programmatic_pages'::regclass
    ) THEN
        ALTER TABLE public.programmatic_pages ADD CONSTRAINT programmatic_pages_directory_id_slug_key UNIQUE (directory_id, slug);
    END IF;
END
$md_bl$;

-- public.programmatic_pages.programmatic_pages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'programmatic_pages_pkey' AND conrelid = 'public.programmatic_pages'::regclass
    ) THEN
        ALTER TABLE public.programmatic_pages ADD CONSTRAINT programmatic_pages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.provider_keys.provider_keys_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_keys_pkey' AND conrelid = 'public.provider_keys'::regclass
    ) THEN
        ALTER TABLE public.provider_keys ADD CONSTRAINT provider_keys_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.provider_keys.provider_keys_tenant_id_provider_label_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_keys_tenant_id_provider_label_key' AND conrelid = 'public.provider_keys'::regclass
    ) THEN
        ALTER TABLE public.provider_keys ADD CONSTRAINT provider_keys_tenant_id_provider_label_key UNIQUE (tenant_id, provider, label);
    END IF;
END
$md_bl$;

-- public.public_pages.public_pages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'public_pages_pkey' AND conrelid = 'public.public_pages'::regclass
    ) THEN
        ALTER TABLE public.public_pages ADD CONSTRAINT public_pages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.public_themes.public_themes_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'public_themes_pkey' AND conrelid = 'public.public_themes'::regclass
    ) THEN
        ALTER TABLE public.public_themes ADD CONSTRAINT public_themes_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.public_themes.public_themes_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'public_themes_slug_key' AND conrelid = 'public.public_themes'::regclass
    ) THEN
        ALTER TABLE public.public_themes ADD CONSTRAINT public_themes_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.referrals.referrals_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'referrals_pkey' AND conrelid = 'public.referrals'::regclass
    ) THEN
        ALTER TABLE public.referrals ADD CONSTRAINT referrals_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.referrals.referrals_referral_code_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'referrals_referral_code_key' AND conrelid = 'public.referrals'::regclass
    ) THEN
        ALTER TABLE public.referrals ADD CONSTRAINT referrals_referral_code_key UNIQUE (referral_code);
    END IF;
END
$md_bl$;

-- public.reviews.reviews_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reviews_pkey' AND conrelid = 'public.reviews'::regclass
    ) THEN
        ALTER TABLE public.reviews ADD CONSTRAINT reviews_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.rfq_bids.rfq_bids_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfq_bids_pkey' AND conrelid = 'public.rfq_bids'::regclass
    ) THEN
        ALTER TABLE public.rfq_bids ADD CONSTRAINT rfq_bids_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.rfq_messages.rfq_messages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfq_messages_pkey' AND conrelid = 'public.rfq_messages'::regclass
    ) THEN
        ALTER TABLE public.rfq_messages ADD CONSTRAINT rfq_messages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.rfqs.rfqs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfqs_pkey' AND conrelid = 'public.rfqs'::regclass
    ) THEN
        ALTER TABLE public.rfqs ADD CONSTRAINT rfqs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.schema_config.schema_config_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'schema_config_directory_id_key' AND conrelid = 'public.schema_config'::regclass
    ) THEN
        ALTER TABLE public.schema_config ADD CONSTRAINT schema_config_directory_id_key UNIQUE (directory_id);
    END IF;
END
$md_bl$;

-- public.schema_config.schema_config_directory_id_schema_type_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'schema_config_directory_id_schema_type_key' AND conrelid = 'public.schema_config'::regclass
    ) THEN
        ALTER TABLE public.schema_config ADD CONSTRAINT schema_config_directory_id_schema_type_key UNIQUE (directory_id, schema_type);
    END IF;
END
$md_bl$;

-- public.schema_config.schema_config_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'schema_config_pkey' AND conrelid = 'public.schema_config'::regclass
    ) THEN
        ALTER TABLE public.schema_config ADD CONSTRAINT schema_config_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.search_config.search_config_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'search_config_directory_id_key' AND conrelid = 'public.search_config'::regclass
    ) THEN
        ALTER TABLE public.search_config ADD CONSTRAINT search_config_directory_id_key UNIQUE (directory_id);
    END IF;
END
$md_bl$;

-- public.search_config.search_config_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'search_config_pkey' AND conrelid = 'public.search_config'::regclass
    ) THEN
        ALTER TABLE public.search_config ADD CONSTRAINT search_config_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.seo_fallback_templates.seo_fallback_templates_directory_id_page_type_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seo_fallback_templates_directory_id_page_type_key' AND conrelid = 'public.seo_fallback_templates'::regclass
    ) THEN
        ALTER TABLE public.seo_fallback_templates ADD CONSTRAINT seo_fallback_templates_directory_id_page_type_key UNIQUE (directory_id, page_type);
    END IF;
END
$md_bl$;

-- public.seo_fallback_templates.seo_fallback_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seo_fallback_templates_pkey' AND conrelid = 'public.seo_fallback_templates'::regclass
    ) THEN
        ALTER TABLE public.seo_fallback_templates ADD CONSTRAINT seo_fallback_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.seo_meta.seo_meta_page_type_page_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seo_meta_page_type_page_id_key' AND conrelid = 'public.seo_meta'::regclass
    ) THEN
        ALTER TABLE public.seo_meta ADD CONSTRAINT seo_meta_page_type_page_id_key UNIQUE (page_type, page_id);
    END IF;
END
$md_bl$;

-- public.seo_meta.seo_meta_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seo_meta_pkey' AND conrelid = 'public.seo_meta'::regclass
    ) THEN
        ALTER TABLE public.seo_meta ADD CONSTRAINT seo_meta_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.service_bookings.service_bookings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_bookings_pkey' AND conrelid = 'public.service_bookings'::regclass
    ) THEN
        ALTER TABLE public.service_bookings ADD CONSTRAINT service_bookings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.service_prices.service_prices_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_prices_pkey' AND conrelid = 'public.service_prices'::regclass
    ) THEN
        ALTER TABLE public.service_prices ADD CONSTRAINT service_prices_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.settlement_invoices.settlement_invoices_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_invoices_pkey' AND conrelid = 'public.settlement_invoices'::regclass
    ) THEN
        ALTER TABLE public.settlement_invoices ADD CONSTRAINT settlement_invoices_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.settlement_invoices.settlement_invoices_run_business_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_invoices_run_business_key' AND conrelid = 'public.settlement_invoices'::regclass
    ) THEN
        ALTER TABLE public.settlement_invoices ADD CONSTRAINT settlement_invoices_run_business_key UNIQUE (run_id, business_id);
    END IF;
END
$md_bl$;

-- public.settlement_payouts.settlement_payouts_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_payouts_pkey' AND conrelid = 'public.settlement_payouts'::regclass
    ) THEN
        ALTER TABLE public.settlement_payouts ADD CONSTRAINT settlement_payouts_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.settlement_payouts.settlement_payouts_run_business_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_payouts_run_business_key' AND conrelid = 'public.settlement_payouts'::regclass
    ) THEN
        ALTER TABLE public.settlement_payouts ADD CONSTRAINT settlement_payouts_run_business_key UNIQUE (run_id, business_id);
    END IF;
END
$md_bl$;

-- public.settlement_runs.settlement_runs_network_period_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_runs_network_period_key' AND conrelid = 'public.settlement_runs'::regclass
    ) THEN
        ALTER TABLE public.settlement_runs ADD CONSTRAINT settlement_runs_network_period_key UNIQUE (network_id, period_start, period_end);
    END IF;
END
$md_bl$;

-- public.settlement_runs.settlement_runs_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_runs_pkey' AND conrelid = 'public.settlement_runs'::regclass
    ) THEN
        ALTER TABLE public.settlement_runs ADD CONSTRAINT settlement_runs_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.shared_leads.shared_leads_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'shared_leads_pkey' AND conrelid = 'public.shared_leads'::regclass
    ) THEN
        ALTER TABLE public.shared_leads ADD CONSTRAINT shared_leads_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.sitemap_config.sitemap_config_directory_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sitemap_config_directory_id_key' AND conrelid = 'public.sitemap_config'::regclass
    ) THEN
        ALTER TABLE public.sitemap_config ADD CONSTRAINT sitemap_config_directory_id_key UNIQUE (directory_id);
    END IF;
END
$md_bl$;

-- public.sitemap_config.sitemap_config_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sitemap_config_pkey' AND conrelid = 'public.sitemap_config'::regclass
    ) THEN
        ALTER TABLE public.sitemap_config ADD CONSTRAINT sitemap_config_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.sponsored_listings.sponsored_listings_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsored_listings_pkey' AND conrelid = 'public.sponsored_listings'::regclass
    ) THEN
        ALTER TABLE public.sponsored_listings ADD CONSTRAINT sponsored_listings_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.sponsors.sponsors_directory_id_business_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsors_directory_id_business_id_key' AND conrelid = 'public.sponsors'::regclass
    ) THEN
        ALTER TABLE public.sponsors ADD CONSTRAINT sponsors_directory_id_business_id_key UNIQUE (directory_id, business_id);
    END IF;
END
$md_bl$;

-- public.sponsors.sponsors_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsors_pkey' AND conrelid = 'public.sponsors'::regclass
    ) THEN
        ALTER TABLE public.sponsors ADD CONSTRAINT sponsors_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.submissions.submissions_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'submissions_pkey' AND conrelid = 'public.submissions'::regclass
    ) THEN
        ALTER TABLE public.submissions ADD CONSTRAINT submissions_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.supplier_products.supplier_products_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'supplier_products_pkey' AND conrelid = 'public.supplier_products'::regclass
    ) THEN
        ALTER TABLE public.supplier_products ADD CONSTRAINT supplier_products_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.survey_responses.survey_responses_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'survey_responses_pkey' AND conrelid = 'public.survey_responses'::regclass
    ) THEN
        ALTER TABLE public.survey_responses ADD CONSTRAINT survey_responses_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.tag_rules.tag_rules_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tag_rules_pkey' AND conrelid = 'public.tag_rules'::regclass
    ) THEN
        ALTER TABLE public.tag_rules ADD CONSTRAINT tag_rules_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.template_categories.template_categories_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'template_categories_pkey' AND conrelid = 'public.template_categories'::regclass
    ) THEN
        ALTER TABLE public.template_categories ADD CONSTRAINT template_categories_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.template_categories.template_categories_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'template_categories_slug_key' AND conrelid = 'public.template_categories'::regclass
    ) THEN
        ALTER TABLE public.template_categories ADD CONSTRAINT template_categories_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.tenants.tenants_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tenants_pkey' AND conrelid = 'public.tenants'::regclass
    ) THEN
        ALTER TABLE public.tenants ADD CONSTRAINT tenants_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.tenants.tenants_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tenants_slug_key' AND conrelid = 'public.tenants'::regclass
    ) THEN
        ALTER TABLE public.tenants ADD CONSTRAINT tenants_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.topic_format_templates.topic_format_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'topic_format_templates_pkey' AND conrelid = 'public.topic_format_templates'::regclass
    ) THEN
        ALTER TABLE public.topic_format_templates ADD CONSTRAINT topic_format_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.tracked_links.tracked_links_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tracked_links_pkey' AND conrelid = 'public.tracked_links'::regclass
    ) THEN
        ALTER TABLE public.tracked_links ADD CONSTRAINT tracked_links_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.tracked_links.tracked_links_short_code_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tracked_links_short_code_key' AND conrelid = 'public.tracked_links'::regclass
    ) THEN
        ALTER TABLE public.tracked_links ADD CONSTRAINT tracked_links_short_code_key UNIQUE (short_code);
    END IF;
END
$md_bl$;

-- public.trap_door_templates.trap_door_templates_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'trap_door_templates_pkey' AND conrelid = 'public.trap_door_templates'::regclass
    ) THEN
        ALTER TABLE public.trap_door_templates ADD CONSTRAINT trap_door_templates_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.twilio_numbers.twilio_numbers_phone_number_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'twilio_numbers_phone_number_key' AND conrelid = 'public.twilio_numbers'::regclass
    ) THEN
        ALTER TABLE public.twilio_numbers ADD CONSTRAINT twilio_numbers_phone_number_key UNIQUE (phone_number);
    END IF;
END
$md_bl$;

-- public.twilio_numbers.twilio_numbers_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'twilio_numbers_pkey' AND conrelid = 'public.twilio_numbers'::regclass
    ) THEN
        ALTER TABLE public.twilio_numbers ADD CONSTRAINT twilio_numbers_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.service_prices.unique_service_per_scope
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'unique_service_per_scope' AND conrelid = 'public.service_prices'::regclass
    ) THEN
        ALTER TABLE public.service_prices ADD CONSTRAINT unique_service_per_scope UNIQUE (directory_id, network_id, service_key);
    END IF;
END
$md_bl$;

-- public.google_places_cache.uq_places_cache_place_id
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'uq_places_cache_place_id' AND conrelid = 'public.google_places_cache'::regclass
    ) THEN
        ALTER TABLE public.google_places_cache ADD CONSTRAINT uq_places_cache_place_id UNIQUE (place_id);
    END IF;
END
$md_bl$;

-- public.users.users_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'users_pkey' AND conrelid = 'public.users'::regclass
    ) THEN
        ALTER TABLE public.users ADD CONSTRAINT users_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.users.users_tenant_id_email_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'users_tenant_id_email_key' AND conrelid = 'public.users'::regclass
    ) THEN
        ALTER TABLE public.users ADD CONSTRAINT users_tenant_id_email_key UNIQUE (tenant_id, email);
    END IF;
END
$md_bl$;

-- public.visitor_accounts.visitor_accounts_email_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_accounts_email_key' AND conrelid = 'public.visitor_accounts'::regclass
    ) THEN
        ALTER TABLE public.visitor_accounts ADD CONSTRAINT visitor_accounts_email_key UNIQUE (email);
    END IF;
END
$md_bl$;

-- public.visitor_accounts.visitor_accounts_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_accounts_pkey' AND conrelid = 'public.visitor_accounts'::regclass
    ) THEN
        ALTER TABLE public.visitor_accounts ADD CONSTRAINT visitor_accounts_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.visitor_events.visitor_events_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_events_pkey' AND conrelid = 'public.visitor_events'::regclass
    ) THEN
        ALTER TABLE public.visitor_events ADD CONSTRAINT visitor_events_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.visitor_favorites.visitor_favorites_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_favorites_pkey' AND conrelid = 'public.visitor_favorites'::regclass
    ) THEN
        ALTER TABLE public.visitor_favorites ADD CONSTRAINT visitor_favorites_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.visitor_favorites.visitor_favorites_visitor_account_id_business_id_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_favorites_visitor_account_id_business_id_key' AND conrelid = 'public.visitor_favorites'::regclass
    ) THEN
        ALTER TABLE public.visitor_favorites ADD CONSTRAINT visitor_favorites_visitor_account_id_business_id_key UNIQUE (visitor_account_id, business_id);
    END IF;
END
$md_bl$;

-- public.visitor_sessions.visitor_sessions_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_sessions_pkey' AND conrelid = 'public.visitor_sessions'::regclass
    ) THEN
        ALTER TABLE public.visitor_sessions ADD CONSTRAINT visitor_sessions_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.visitors.visitors_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitors_pkey' AND conrelid = 'public.visitors'::regclass
    ) THEN
        ALTER TABLE public.visitors ADD CONSTRAINT visitors_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.webhook_deliveries.webhook_deliveries_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'webhook_deliveries_pkey' AND conrelid = 'public.webhook_deliveries'::regclass
    ) THEN
        ALTER TABLE public.webhook_deliveries ADD CONSTRAINT webhook_deliveries_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.webhooks.webhooks_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'webhooks_pkey' AND conrelid = 'public.webhooks'::regclass
    ) THEN
        ALTER TABLE public.webhooks ADD CONSTRAINT webhooks_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.zaarhub_legal_pages.zaarhub_legal_pages_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'zaarhub_legal_pages_pkey' AND conrelid = 'public.zaarhub_legal_pages'::regclass
    ) THEN
        ALTER TABLE public.zaarhub_legal_pages ADD CONSTRAINT zaarhub_legal_pages_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

-- public.zaarhub_legal_pages.zaarhub_legal_pages_slug_key
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'zaarhub_legal_pages_slug_key' AND conrelid = 'public.zaarhub_legal_pages'::regclass
    ) THEN
        ALTER TABLE public.zaarhub_legal_pages ADD CONSTRAINT zaarhub_legal_pages_slug_key UNIQUE (slug);
    END IF;
END
$md_bl$;

-- public.zaarhub_site_config.zaarhub_site_config_pkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'zaarhub_site_config_pkey' AND conrelid = 'public.zaarhub_site_config'::regclass
    ) THEN
        ALTER TABLE public.zaarhub_site_config ADD CONSTRAINT zaarhub_site_config_pkey PRIMARY KEY (id);
    END IF;
END
$md_bl$;

CREATE UNIQUE INDEX IF NOT EXISTS discovery_queue_nameaddr_uniq ON public.discovery_queue USING btree (directory_id, lower(name), lower(COALESCE(address, ''::text)));

CREATE UNIQUE INDEX IF NOT EXISTS discovery_queue_place_uniq ON public.discovery_queue USING btree (directory_id, place_id) WHERE (place_id IS NOT NULL);

CREATE UNIQUE INDEX IF NOT EXISTS domain_mappings_host_path ON public.domain_mappings USING btree (domain, url_path);

CREATE INDEX IF NOT EXISTS idx__city_tags_directory_id ON public._city_tags USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_account_links_email ON public.account_links USING btree (email);

CREATE INDEX IF NOT EXISTS idx_account_links_user_id ON public.account_links USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_account_links_visitor_account_id ON public.account_links USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_ad_creatives_dims ON public.ad_creatives USING btree (width, height);

CREATE INDEX IF NOT EXISTS idx_ad_creatives_sponsor ON public.ad_creatives USING btree (sponsor_id);

CREATE INDEX IF NOT EXISTS idx_ad_creatives_sponsor_id ON public.ad_creatives USING btree (sponsor_id);

CREATE INDEX IF NOT EXISTS idx_ad_creatives_status ON public.ad_creatives USING btree (status);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_ad_zone_id ON public.ad_earnings USING btree (ad_zone_id);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_directory ON public.ad_earnings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_directory_id ON public.ad_earnings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_schedule_id ON public.ad_earnings USING btree (schedule_id);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_sponsor ON public.ad_earnings USING btree (sponsor_id);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_sponsor_id ON public.ad_earnings USING btree (sponsor_id);

CREATE INDEX IF NOT EXISTS idx_ad_earnings_zone ON public.ad_earnings USING btree (ad_zone_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_ad_zone_id ON public.ad_schedules USING btree (ad_zone_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_creative_id ON public.ad_schedules USING btree (creative_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_dates ON public.ad_schedules USING btree (start_date, end_date);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_directory ON public.ad_schedules USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_directory_id ON public.ad_schedules USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_sponsor ON public.ad_schedules USING btree (sponsor_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_sponsor_id ON public.ad_schedules USING btree (sponsor_id);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_status ON public.ad_schedules USING btree (status);

CREATE INDEX IF NOT EXISTS idx_ad_schedules_zone ON public.ad_schedules USING btree (ad_zone_id);

CREATE INDEX IF NOT EXISTS idx_ad_zones_directory_id ON public.ad_zones USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_analytics_created_at ON public.analytics_events USING btree (created_at);

CREATE INDEX IF NOT EXISTS idx_analytics_directory ON public.analytics_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_analytics_event_type ON public.analytics_events USING btree (event_type);

CREATE INDEX IF NOT EXISTS idx_analytics_events_directory_id ON public.analytics_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_api_key_usage_api_key_id ON public.api_key_usage USING btree (api_key_id);

CREATE INDEX IF NOT EXISTS idx_api_key_usage_created ON public.api_key_usage USING btree (created_at);

CREATE INDEX IF NOT EXISTS idx_api_key_usage_key ON public.api_key_usage USING btree (api_key_id);

CREATE INDEX IF NOT EXISTS idx_api_keys_key_hash ON public.api_keys USING btree (key_hash);

CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON public.api_keys USING btree (key_prefix);

CREATE INDEX IF NOT EXISTS idx_api_keys_tenant_id ON public.api_keys USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_api_keys_user ON public.api_keys USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON public.api_keys USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_approval_queue_directory ON public.approval_queue USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_approval_queue_directory_id ON public.approval_queue USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_approval_queue_status ON public.approval_queue USING btree (status);

CREATE INDEX IF NOT EXISTS idx_author_profiles_dir ON public.author_profiles USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_author_profiles_directory_id ON public.author_profiles USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_author_profiles_user_id ON public.author_profiles USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_b2b_notifications_business ON public.b2b_notifications USING btree (business_id, is_read);

CREATE INDEX IF NOT EXISTS idx_b2b_notifications_business_id ON public.b2b_notifications USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_b2b_notifications_created ON public.b2b_notifications USING btree (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_b2b_notifications_related_message_id ON public.b2b_notifications USING btree (related_message_id);

CREATE INDEX IF NOT EXISTS idx_b2b_notifications_related_order_id ON public.b2b_notifications USING btree (related_order_id);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_buyer ON public.b2b_orders USING btree (buyer_business_id);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_buyer_business_id ON public.b2b_orders USING btree (buyer_business_id);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_product_id ON public.b2b_orders USING btree (product_id);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_status ON public.b2b_orders USING btree (status);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_supplier ON public.b2b_orders USING btree (supplier_business_id, status);

CREATE INDEX IF NOT EXISTS idx_b2b_orders_supplier_business_id ON public.b2b_orders USING btree (supplier_business_id);

CREATE INDEX IF NOT EXISTS idx_biz_cats_business ON public.business_categories USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_biz_cats_category ON public.business_categories USING btree (category_id);

CREATE INDEX IF NOT EXISTS idx_blog_media_blog_post_id ON public.blog_media USING btree (blog_post_id);

CREATE INDEX IF NOT EXISTS idx_blog_media_post ON public.blog_media USING btree (blog_post_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_author_id ON public.blog_posts USING btree (author_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_decay_flag ON public.blog_posts USING btree (decay_flag);

CREATE INDEX IF NOT EXISTS idx_blog_posts_directory ON public.blog_posts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_directory_id ON public.blog_posts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_location_id ON public.blog_posts USING btree (location_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_master_post_id ON public.blog_posts USING btree (master_post_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_mentioned_biz ON public.blog_posts USING gin (mentioned_business_ids);

CREATE INDEX IF NOT EXISTS idx_blog_posts_published ON public.blog_posts USING btree (published);

CREATE INDEX IF NOT EXISTS idx_blog_posts_refresh_priority ON public.blog_posts USING btree (refresh_priority);

CREATE INDEX IF NOT EXISTS idx_blog_posts_service_id ON public.blog_posts USING btree (service_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_status ON public.blog_posts USING btree (status);

CREATE INDEX IF NOT EXISTS idx_blog_posts_template_id ON public.blog_posts USING btree (template_id);

CREATE INDEX IF NOT EXISTS idx_blog_posts_type ON public.blog_posts USING btree (post_type);

CREATE INDEX IF NOT EXISTS idx_blog_qa_keywords_directory_id ON public.blog_qa_keywords USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_qa_posts_blog_post_id ON public.blog_qa_posts USING btree (blog_post_id);

CREATE INDEX IF NOT EXISTS idx_blog_qa_posts_directory_id ON public.blog_qa_posts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_template_directories_directory_id ON public.blog_template_directories USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_template_directories_template_id ON public.blog_template_directories USING btree (template_id);

CREATE INDEX IF NOT EXISTS idx_blog_templates_category ON public.blog_templates USING btree (category);

CREATE INDEX IF NOT EXISTS idx_blog_templates_dir ON public.blog_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_templates_directory ON public.blog_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_templates_directory_id ON public.blog_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_tmpl_dirs_directory ON public.blog_template_directories USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_blog_tmpl_dirs_template ON public.blog_template_directories USING btree (template_id);

CREATE INDEX IF NOT EXISTS idx_bpl_network_month ON public.business_point_ledger USING btree (network_id, month_key);

CREATE INDEX IF NOT EXISTS idx_bundle_services_bundle_id ON public.bundle_services USING btree (bundle_id);

CREATE INDEX IF NOT EXISTS idx_business_articles_biz_id ON public.business_articles USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_articles_business_id ON public.business_articles USING btree (business_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_business_articles_dir_slug ON public.business_articles USING btree (directory_id, slug);

CREATE INDEX IF NOT EXISTS idx_business_articles_directory ON public.business_articles USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_business_articles_directory_id ON public.business_articles USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_business_articles_slug ON public.business_articles USING btree (slug);

CREATE INDEX IF NOT EXISTS idx_business_categories_business ON public.business_categories USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_categories_business_id ON public.business_categories USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_categories_category ON public.business_categories USING btree (category_id);

CREATE INDEX IF NOT EXISTS idx_business_categories_category_id ON public.business_categories USING btree (category_id);

CREATE INDEX IF NOT EXISTS idx_business_listings_category ON public.business_listings USING btree (category);

CREATE INDEX IF NOT EXISTS idx_business_listings_city ON public.business_listings USING btree (city_page_id);

CREATE INDEX IF NOT EXISTS idx_business_listings_city_page_id ON public.business_listings USING btree (city_page_id);

CREATE INDEX IF NOT EXISTS idx_business_listings_editors_pick ON public.business_listings USING btree (is_editors_pick) WHERE (is_editors_pick = true);

CREATE INDEX IF NOT EXISTS idx_business_listings_featured ON public.business_listings USING btree (is_featured) WHERE (is_featured = true);

CREATE INDEX IF NOT EXISTS idx_business_listings_name ON public.business_listings USING btree (business_name);

CREATE INDEX IF NOT EXISTS idx_business_listings_rating ON public.business_listings USING btree (rating DESC);

CREATE INDEX IF NOT EXISTS idx_business_listings_search ON public.business_listings USING gin (to_tsvector('english'::regconfig, (((((COALESCE(business_name, ''::character varying))::text || ' '::text) || COALESCE(description, ''::text)) || ' '::text) || (COALESCE(category, ''::character varying))::text)));

CREATE INDEX IF NOT EXISTS idx_business_messages_business_id ON public.business_messages USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_messages_is_read ON public.business_messages USING btree (is_read);

CREATE INDEX IF NOT EXISTS idx_business_messages_sender_business ON public.business_messages USING btree (sender_business_id);

CREATE INDEX IF NOT EXISTS idx_business_messages_sender_business_id ON public.business_messages USING btree (sender_business_id);

CREATE INDEX IF NOT EXISTS idx_business_messages_to_business ON public.business_messages USING btree (to_business_id);

CREATE INDEX IF NOT EXISTS idx_business_messages_to_business_id ON public.business_messages USING btree (to_business_id);

CREATE INDEX IF NOT EXISTS idx_business_meta_business ON public.business_meta USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_meta_business_id ON public.business_meta USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_meta_template ON public.business_meta USING btree (template);

CREATE INDEX IF NOT EXISTS idx_business_point_ledger_network_id ON public.business_point_ledger USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_business_services_active ON public.business_services USING btree (business_id) WHERE (is_active = true);

CREATE INDEX IF NOT EXISTS idx_business_services_business ON public.business_services USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_services_business_id ON public.business_services USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_services_directory ON public.business_services USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_business_services_directory_id ON public.business_services USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_business_subscriptions_tier_id ON public.business_subscriptions USING btree (tier_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_events_actor_user_id ON public.business_transfer_events USING btree (actor_user_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_events_business_id ON public.business_transfer_events USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_events_created_at ON public.business_transfer_events USING btree (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_business_transfer_events_transfer_id ON public.business_transfer_events USING btree (transfer_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_business_id ON public.business_transfer_fees USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_payee_user_id ON public.business_transfer_fees USING btree (payee_user_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_payer_user_id ON public.business_transfer_fees USING btree (payer_user_id);

CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_status ON public.business_transfer_fees USING btree (status);

CREATE INDEX IF NOT EXISTS idx_business_transfer_fees_transfer_id ON public.business_transfer_fees USING btree (transfer_id);

CREATE INDEX IF NOT EXISTS idx_business_transfers_business_id ON public.business_transfers USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_transfers_created_at ON public.business_transfers USING btree (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_business_transfers_from_email_lower ON public.business_transfers USING btree (lower(from_email));

CREATE INDEX IF NOT EXISTS idx_business_transfers_from_tenant_id ON public.business_transfers USING btree (from_tenant_id);

CREATE INDEX IF NOT EXISTS idx_business_transfers_from_user_id ON public.business_transfers USING btree (from_user_id);

CREATE INDEX IF NOT EXISTS idx_business_transfers_requested_by ON public.business_transfers USING btree (requested_by);

CREATE INDEX IF NOT EXISTS idx_business_transfers_status ON public.business_transfers USING btree (status);

CREATE INDEX IF NOT EXISTS idx_business_transfers_target_directory_id ON public.business_transfers USING btree (target_directory_id);

CREATE INDEX IF NOT EXISTS idx_business_transfers_to_email_lower ON public.business_transfers USING btree (lower(to_email));

CREATE INDEX IF NOT EXISTS idx_business_transfers_to_tenant_id ON public.business_transfers USING btree (to_tenant_id);

CREATE INDEX IF NOT EXISTS idx_business_transfers_to_user_id ON public.business_transfers USING btree (to_user_id);

CREATE INDEX IF NOT EXISTS idx_business_verifications_business ON public.business_verifications USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_verifications_business_id ON public.business_verifications USING btree (business_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_business_verifications_business_unique ON public.business_verifications USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_business_verifications_directory_id ON public.business_verifications USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_business_verifications_status ON public.business_verifications USING btree (status);

CREATE INDEX IF NOT EXISTS idx_business_verifications_verified_by ON public.business_verifications USING btree (verified_by);

CREATE INDEX IF NOT EXISTS idx_businesses_category ON public.businesses USING btree (category_id);

CREATE INDEX IF NOT EXISTS idx_businesses_category_directory ON public.businesses USING btree (category_id, directory_id);

CREATE INDEX IF NOT EXISTS idx_businesses_directory ON public.businesses USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_businesses_directory_id ON public.businesses USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_businesses_enriched_at ON public.businesses USING btree (enriched_at);

CREATE INDEX IF NOT EXISTS idx_businesses_featured_deal_id ON public.businesses USING btree (featured_deal_id);

CREATE INDEX IF NOT EXISTS idx_businesses_featured_product_id ON public.businesses USING btree (featured_product_id);

CREATE INDEX IF NOT EXISTS idx_businesses_is_franchise ON public.businesses USING btree (is_franchise) WHERE (is_franchise = true);

CREATE INDEX IF NOT EXISTS idx_businesses_location ON public.businesses USING btree (latitude, longitude);

CREATE INDEX IF NOT EXISTS idx_businesses_search ON public.businesses USING gin (to_tsvector('english'::regconfig, (((name)::text || ' '::text) || COALESCE(description, ''::text))));

CREATE INDEX IF NOT EXISTS idx_businesses_search_vector ON public.businesses USING gin (search_vector);

CREATE INDEX IF NOT EXISTS idx_businesses_type ON public.businesses USING btree (business_type);

CREATE INDEX IF NOT EXISTS idx_buying_group_deals_group ON public.buying_group_deals USING btree (group_id);

CREATE INDEX IF NOT EXISTS idx_buying_group_deals_group_id ON public.buying_group_deals USING btree (group_id);

CREATE INDEX IF NOT EXISTS idx_buying_group_deals_status ON public.buying_group_deals USING btree (status);

CREATE INDEX IF NOT EXISTS idx_buying_group_deals_supplier_business_id ON public.buying_group_deals USING btree (supplier_business_id);

CREATE INDEX IF NOT EXISTS idx_buying_group_members_business ON public.buying_group_members USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_buying_group_members_business_id ON public.buying_group_members USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_buying_group_members_group ON public.buying_group_members USING btree (group_id);

CREATE INDEX IF NOT EXISTS idx_buying_group_members_group_id ON public.buying_group_members USING btree (group_id);

CREATE INDEX IF NOT EXISTS idx_buying_groups_category ON public.buying_groups USING btree (category);

CREATE INDEX IF NOT EXISTS idx_buying_groups_founder ON public.buying_groups USING btree (founder_business_id);

CREATE INDEX IF NOT EXISTS idx_buying_groups_founder_business_id ON public.buying_groups USING btree (founder_business_id);

CREATE INDEX IF NOT EXISTS idx_buying_groups_status ON public.buying_groups USING btree (status);

CREATE INDEX IF NOT EXISTS idx_call_logs_directory_id ON public.call_logs USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_category_redeem_caps_network_id ON public.category_redeem_caps USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_category_requests_business ON public.category_requests USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_category_requests_business_id ON public.category_requests USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_category_requests_category_id ON public.category_requests USING btree (category_id);

CREATE INDEX IF NOT EXISTS idx_category_requests_status ON public.category_requests USING btree (status);

CREATE UNIQUE INDEX IF NOT EXISTS idx_category_requests_unique ON public.category_requests USING btree (business_id, category_id) WHERE (status = 'pending'::text);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_business ON public.checkout_sessions USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_business_id ON public.checkout_sessions USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_directory_id ON public.checkout_sessions USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_provider ON public.checkout_sessions USING btree (provider_session_id);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_status ON public.checkout_sessions USING btree (status);

CREATE INDEX IF NOT EXISTS idx_city_pages_active ON public.city_pages USING btree (is_active) WHERE (is_active = true);

CREATE UNIQUE INDEX IF NOT EXISTS idx_city_pages_slug ON public.city_pages USING btree (tenant_id, city_slug);

CREATE INDEX IF NOT EXISTS idx_city_pages_tenant ON public.city_pages USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_city_plan_slots_plan_tier_id ON public.city_plan_slots USING btree (plan_tier_id);

CREATE INDEX IF NOT EXISTS idx_city_requests_directory_id ON public.city_requests USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_city_requests_status ON public.city_requests USING btree (status);

CREATE INDEX IF NOT EXISTS idx_city_requests_votes ON public.city_requests USING btree (votes DESC);

CREATE INDEX IF NOT EXISTS idx_claim_offers_active ON public.claim_offers USING btree (is_active) WHERE (is_active = true);

CREATE INDEX IF NOT EXISTS idx_claim_offers_listing ON public.claim_offers USING btree (listing_id);

CREATE INDEX IF NOT EXISTS idx_claim_offers_listing_id ON public.claim_offers USING btree (listing_id);

CREATE INDEX IF NOT EXISTS idx_claim_offers_type ON public.claim_offers USING btree (offer_type);

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_business ON public.claimed_businesses USING btree (business_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_claimed_businesses_business_id ON public.claimed_businesses USING btree (business_id) WHERE (business_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_email ON public.claimed_businesses USING btree (owner_email);

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_owner_email ON public.claimed_businesses USING btree (owner_email);

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_subscription_id ON public.claimed_businesses USING btree (subscription_id);

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_user_id ON public.claimed_businesses USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_claimed_businesses_visitor ON public.claimed_businesses USING btree (visitor_account_id) WHERE (visitor_account_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_community_events_business_id ON public.community_events USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_community_events_created_by ON public.community_events USING btree (created_by);

CREATE INDEX IF NOT EXISTS idx_community_events_date ON public.community_events USING btree (event_date);

CREATE INDEX IF NOT EXISTS idx_community_events_directory ON public.community_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_community_events_directory_id ON public.community_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_community_events_loyalty ON public.community_events USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_community_events_loyalty_program_id ON public.community_events USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_community_events_source_provider_id ON public.community_events USING btree (source_provider_id);

CREATE INDEX IF NOT EXISTS idx_community_events_status ON public.community_events USING btree (status);

CREATE INDEX IF NOT EXISTS idx_connected_services_active ON public.connected_services USING btree (service, is_active);

CREATE INDEX IF NOT EXISTS idx_connected_services_user ON public.connected_services USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_content_queue_directory_id ON public.content_queue USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_content_queue_scheduled ON public.content_queue USING btree (scheduled_for);

CREATE INDEX IF NOT EXISTS idx_content_queue_status ON public.content_queue USING btree (status);

CREATE INDEX IF NOT EXISTS idx_content_research_directory_id ON public.content_research USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_content_research_topic_id ON public.content_research USING btree (topic_id);

CREATE INDEX IF NOT EXISTS idx_content_topics_dir ON public.content_topics USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_content_topics_directory_id ON public.content_topics USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_content_topics_location_id ON public.content_topics USING btree (location_id);

CREATE INDEX IF NOT EXISTS idx_content_topics_service_id ON public.content_topics USING btree (service_id);

CREATE INDEX IF NOT EXISTS idx_content_topics_status ON public.content_topics USING btree (status);

CREATE INDEX IF NOT EXISTS idx_crm_contacts_directory ON public.crm_contacts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_crm_contacts_directory_id ON public.crm_contacts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_crm_contacts_email ON public.crm_contacts USING btree (email);

CREATE INDEX IF NOT EXISTS idx_crm_deal_records_contact_id ON public.crm_deal_records USING btree (contact_id);

CREATE INDEX IF NOT EXISTS idx_crm_deal_records_directory_id ON public.crm_deal_records USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_crm_deal_records_pipeline_id ON public.crm_deal_records USING btree (pipeline_id);

CREATE INDEX IF NOT EXISTS idx_crm_pipelines_directory_id ON public.crm_pipelines USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_data_enrichment_logs_business ON public.data_enrichment_logs USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_data_enrichment_logs_business_id ON public.data_enrichment_logs USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_data_enrichment_logs_directory_id ON public.data_enrichment_logs USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_deal_claims_code ON public.deal_claims USING btree (claim_code);

CREATE INDEX IF NOT EXISTS idx_deal_claims_deal_id ON public.deal_claims USING btree (deal_id);

CREATE INDEX IF NOT EXISTS idx_deal_claims_email ON public.deal_claims USING btree (visitor_email);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_business_id ON public.deal_redemptions USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_code ON public.deal_redemptions USING btree (redemption_code);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_deal ON public.deal_redemptions USING btree (deal_id);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_deal_id ON public.deal_redemptions USING btree (deal_id);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_deal_visitor ON public.deal_redemptions USING btree (deal_id, visitor_id);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_visitor ON public.deal_redemptions USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_deal_redemptions_visitor_id ON public.deal_redemptions USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_deals_active ON public.deals USING btree (is_active);

CREATE INDEX IF NOT EXISTS idx_deals_business_id ON public.deals USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_deals_directory_id ON public.deals USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_deals_loyalty_program ON public.deals USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_deals_loyalty_program_id ON public.deals USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_deals_zaarhub_featured ON public.deals USING btree (zaarhub_featured) WHERE (zaarhub_featured = true);

CREATE INDEX IF NOT EXISTS idx_demand_analytics_settings_directory_id ON public.demand_analytics_settings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_dir_email_settings_dir ON public.directory_email_settings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_dir_email_settings_directory ON public.directory_email_settings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_dir_surveys_directory ON public.directory_surveys USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directories_network ON public.directories USING btree (network_id) WHERE (network_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_directories_network_id ON public.directories USING btree (network_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_directories_slug ON public.directories USING btree (slug);

CREATE INDEX IF NOT EXISTS idx_directory_branding_directory_id ON public.directory_branding USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_categories_parent ON public.directory_categories USING btree (parent_id) WHERE (parent_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_directory_categories_parent_id ON public.directory_categories USING btree (parent_id);

CREATE INDEX IF NOT EXISTS idx_directory_email_settings_directory_id ON public.directory_email_settings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_events_actor_id ON public.directory_events USING btree (actor_id);

CREATE INDEX IF NOT EXISTS idx_directory_events_created ON public.directory_events USING btree (created_at);

CREATE INDEX IF NOT EXISTS idx_directory_events_directory ON public.directory_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_events_directory_id ON public.directory_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_events_entity ON public.directory_events USING btree (entity_type, entity_id);

CREATE INDEX IF NOT EXISTS idx_directory_events_tenant_id ON public.directory_events USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_directory_events_type ON public.directory_events USING btree (event_type);

CREATE INDEX IF NOT EXISTS idx_directory_events_unprocessed ON public.directory_events USING btree (processed, created_at) WHERE (processed = false);

CREATE INDEX IF NOT EXISTS idx_directory_locations_dir ON public.directory_locations USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_locations_directory_id ON public.directory_locations USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_notifications_active ON public.directory_notifications USING btree (directory_id, is_active, starts_at, expires_at);

CREATE INDEX IF NOT EXISTS idx_directory_notifications_directory_id ON public.directory_notifications USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_services_dir ON public.directory_services USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_services_directory_id ON public.directory_services USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_surveys_directory_id ON public.directory_surveys USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_surveys_network ON public.directory_surveys USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_directory_tiers_directory ON public.directory_tiers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_tiers_directory_id ON public.directory_tiers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_directory_tiers_plan_tier_id ON public.directory_tiers USING btree (plan_tier_id);

CREATE INDEX IF NOT EXISTS idx_discovery_queue_dir ON public.discovery_queue USING btree (directory_id, status);

CREATE INDEX IF NOT EXISTS idx_discovery_queue_directory_id ON public.discovery_queue USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_discovery_queue_source ON public.discovery_queue USING btree (directory_id, source);

CREATE INDEX IF NOT EXISTS idx_domain_mappings_directory ON public.domain_mappings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_domain_mappings_directory_id ON public.domain_mappings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_domain_mappings_domain ON public.domain_mappings USING btree (domain);

CREATE INDEX IF NOT EXISTS idx_domain_mappings_live ON public.domain_mappings USING btree (status, live_status);

CREATE INDEX IF NOT EXISTS idx_email_campaigns_directory_id ON public.email_campaigns USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_email_campaigns_template_id ON public.email_campaigns USING btree (template_id);

CREATE INDEX IF NOT EXISTS idx_email_templates_directory_id ON public.email_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_enrichment_settings_directory_id ON public.enrichment_settings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_enrichment_settings_next_run_at ON public.enrichment_settings USING btree (next_run_at);

CREATE INDEX IF NOT EXISTS idx_event_providers_directory_id ON public.event_providers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_event_rsvps_event ON public.event_rsvps USING btree (event_id);

CREATE INDEX IF NOT EXISTS idx_event_rsvps_event_id ON public.event_rsvps USING btree (event_id);

CREATE INDEX IF NOT EXISTS idx_event_rsvps_visitor ON public.event_rsvps USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_event_rsvps_visitor_account_id ON public.event_rsvps USING btree (visitor_account_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_events_source ON public.community_events USING btree (directory_id, source_provider_id, source_event_id) WHERE ((source_provider_id IS NOT NULL) AND (source_event_id IS NOT NULL));

CREATE INDEX IF NOT EXISTS idx_events_zaarhub_featured ON public.community_events USING btree (zaarhub_featured) WHERE (zaarhub_featured = true);

CREATE INDEX IF NOT EXISTS idx_export_templates_directory_id ON public.export_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_google_places_cache_place_id ON public.google_places_cache USING btree (place_id);

CREATE INDEX IF NOT EXISTS idx_google_places_cache_query ON public.google_places_cache USING btree (query);

CREATE INDEX IF NOT EXISTS idx_grandfathered_pricing_business_id ON public.grandfathered_pricing USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_group_deal_commitments_business ON public.group_deal_commitments USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_group_deal_commitments_business_id ON public.group_deal_commitments USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_group_deal_commitments_deal ON public.group_deal_commitments USING btree (deal_id);

CREATE INDEX IF NOT EXISTS idx_group_deal_commitments_deal_id ON public.group_deal_commitments USING btree (deal_id);

CREATE INDEX IF NOT EXISTS idx_homepage_sections_directory ON public.homepage_sections USING btree (directory_id) WHERE (directory_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_homepage_sections_directory_id ON public.homepage_sections USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_homepage_sections_network ON public.homepage_sections USING btree (network_id) WHERE (network_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_homepage_sections_network_id ON public.homepage_sections USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_import_logs_directory_id ON public.import_logs USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_landing_pages_directory_id ON public.landing_pages USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_lead_share_transactions_from ON public.lead_share_transactions USING btree (from_business_id);

CREATE INDEX IF NOT EXISTS idx_lead_share_transactions_from_business_id ON public.lead_share_transactions USING btree (from_business_id);

CREATE INDEX IF NOT EXISTS idx_lead_share_transactions_lead ON public.lead_share_transactions USING btree (lead_id);

CREATE INDEX IF NOT EXISTS idx_lead_share_transactions_lead_id ON public.lead_share_transactions USING btree (lead_id);

CREATE INDEX IF NOT EXISTS idx_lead_share_transactions_to ON public.lead_share_transactions USING btree (to_business_id);

CREATE INDEX IF NOT EXISTS idx_lead_share_transactions_to_business_id ON public.lead_share_transactions USING btree (to_business_id);

CREATE INDEX IF NOT EXISTS idx_legal_pages_published ON public.legal_pages USING btree (published);

CREATE INDEX IF NOT EXISTS idx_legal_pages_type ON public.legal_pages USING btree (page_type);

CREATE INDEX IF NOT EXISTS idx_link_clicks_clicked ON public.link_clicks USING btree (clicked_at);

CREATE INDEX IF NOT EXISTS idx_link_clicks_contact ON public.link_clicks USING btree (contact_id);

CREATE INDEX IF NOT EXISTS idx_link_clicks_link ON public.link_clicks USING btree (link_id);

CREATE INDEX IF NOT EXISTS idx_link_clicks_link_id ON public.link_clicks USING btree (link_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_activity_created ON public.loyalty_activity USING btree (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_loyalty_activity_member ON public.loyalty_activity USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_activity_member_id ON public.loyalty_activity USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_checkins_member_id ON public.loyalty_checkins USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_enrollments_program_id ON public.loyalty_enrollments USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_members_network ON public.loyalty_members USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_members_network_id ON public.loyalty_members USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_members_program ON public.loyalty_members USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_members_program_id ON public.loyalty_members USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_members_visitor ON public.loyalty_members USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_members_visitor_account_id ON public.loyalty_members USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_milestones_completed_member_id ON public.loyalty_milestones_completed USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_milestones_completed_milestone_id ON public.loyalty_milestones_completed USING btree (milestone_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_milestones_loyalty_program_id ON public.loyalty_milestones USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_directory ON public.loyalty_programs USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_directory_active ON public.loyalty_programs USING btree (directory_id) WHERE (is_active = true);

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_directory_id ON public.loyalty_programs USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_network ON public.loyalty_programs USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_programs_network_id ON public.loyalty_programs USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_reward_tiers_program_id ON public.loyalty_reward_tiers USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_rewards_earned_member_id ON public.loyalty_rewards_earned USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_rewards_earned_tier_id ON public.loyalty_rewards_earned USING btree (tier_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_rewards_member ON public.loyalty_rewards_earned USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_scans_business_id ON public.loyalty_scans USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_scans_member_id ON public.loyalty_scans USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_scans_program ON public.loyalty_scans USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_scans_program_id ON public.loyalty_scans USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_tiers_loyalty_program_id ON public.loyalty_tiers USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_loyalty_tiers_program ON public.loyalty_tiers USING btree (loyalty_program_id);

CREATE INDEX IF NOT EXISTS idx_network_branding_network_id ON public.network_branding USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_networks_owner_id ON public.networks USING btree (owner_id);

CREATE INDEX IF NOT EXISTS idx_newsletter_digests_directory_id ON public.newsletter_digests USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_newsletter_queue_dir ON public.newsletter_queue USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_newsletter_queue_directory_id ON public.newsletter_queue USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_newsletter_subscribers_dir ON public.newsletter_subscribers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_newsletter_subscribers_directory_id ON public.newsletter_subscribers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_newsletter_subscribers_status ON public.newsletter_subscribers USING btree (status);

CREATE INDEX IF NOT EXISTS idx_nl_subscribers_directory ON public.newsletter_subscribers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_nl_subscribers_status ON public.newsletter_subscribers USING btree (status);

CREATE INDEX IF NOT EXISTS idx_offer_claims_offer ON public.offer_claims USING btree (offer_id);

CREATE INDEX IF NOT EXISTS idx_offer_claims_offer_id ON public.offer_claims USING btree (offer_id);

CREATE INDEX IF NOT EXISTS idx_offer_claims_redeemed ON public.offer_claims USING btree (redeemed) WHERE (redeemed = false);

CREATE INDEX IF NOT EXISTS idx_offer_claims_visitor ON public.offer_claims USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_password_resets_user_id ON public.password_resets USING btree (user_id);

CREATE INDEX IF NOT EXISTS idx_pay_per_call_business_id ON public.pay_per_call USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_pay_per_call_directory_id ON public.pay_per_call USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_payment_providers_active ON public.payment_providers USING btree (is_active) WHERE (is_active = true);

CREATE INDEX IF NOT EXISTS idx_payment_webhook_events_provider ON public.payment_webhook_events USING btree (provider_type);

CREATE INDEX IF NOT EXISTS idx_payment_webhook_events_status ON public.payment_webhook_events USING btree (status);

CREATE INDEX IF NOT EXISTS idx_pil_business ON public.point_issuance_log USING btree (issuing_business_id);

CREATE INDEX IF NOT EXISTS idx_pil_member ON public.point_issuance_log USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_pil_network ON public.point_issuance_log USING btree (network_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_pil_unexpired_issuance ON public.point_issuance_log USING btree (network_id, created_at) WHERE (expired_at IS NULL);

CREATE INDEX IF NOT EXISTS idx_plan_slot_bookings_business_id ON public.plan_slot_bookings USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_plan_slot_bookings_plan_tier_id ON public.plan_slot_bookings USING btree (plan_tier_id);

CREATE INDEX IF NOT EXISTS idx_point_issuance_log_member_id ON public.point_issuance_log USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_point_issuance_log_network_id ON public.point_issuance_log USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_point_issuance_log_program_id ON public.point_issuance_log USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_point_redemption_log_member_id ON public.point_redemption_log USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_point_redemption_log_network_id ON public.point_redemption_log USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_point_redemption_log_program_id ON public.point_redemption_log USING btree (program_id);

CREATE INDEX IF NOT EXISTS idx_point_treasury_network_id ON public.point_treasury USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_poll_votes_poll ON public.poll_votes USING btree (poll_id);

CREATE INDEX IF NOT EXISTS idx_poll_votes_poll_id ON public.poll_votes USING btree (poll_id);

CREATE INDEX IF NOT EXISTS idx_poll_votes_visitor_account_id ON public.poll_votes USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_polls_created_by ON public.polls USING btree (created_by);

CREATE INDEX IF NOT EXISTS idx_polls_directory ON public.polls USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_polls_directory_id ON public.polls USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_polls_status ON public.polls USING btree (status);

CREATE INDEX IF NOT EXISTS idx_price_bundles_directory_id ON public.price_bundles USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_price_bundles_network_id ON public.price_bundles USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_prl_business ON public.point_redemption_log USING btree (redeeming_business_id);

CREATE INDEX IF NOT EXISTS idx_prl_member ON public.point_redemption_log USING btree (member_id);

CREATE INDEX IF NOT EXISTS idx_prl_network ON public.point_redemption_log USING btree (network_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_programmatic_pages_dir ON public.programmatic_pages USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_programmatic_pages_directory_id ON public.programmatic_pages USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_programmatic_pages_location_id ON public.programmatic_pages USING btree (location_id);

CREATE INDEX IF NOT EXISTS idx_programmatic_pages_mentioned_biz ON public.programmatic_pages USING gin (mentioned_business_ids);

CREATE INDEX IF NOT EXISTS idx_programmatic_pages_service_id ON public.programmatic_pages USING btree (service_id);

CREATE INDEX IF NOT EXISTS idx_programmatic_pages_status ON public.programmatic_pages USING btree (status);

CREATE INDEX IF NOT EXISTS idx_provider_keys_directory ON public.provider_keys USING btree (directory_id, provider);

CREATE INDEX IF NOT EXISTS idx_provider_keys_network ON public.provider_keys USING btree (network_id, provider);

CREATE INDEX IF NOT EXISTS idx_provider_keys_resolve ON public.provider_keys USING btree (tenant_id, provider, is_default DESC, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_provider_keys_tenant ON public.provider_keys USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_provider_keys_tenant_id ON public.provider_keys USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_public_pages_business_id ON public.public_pages USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_public_pages_created_at ON public.public_pages USING btree (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_public_pages_directory_id ON public.public_pages USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_public_pages_featured ON public.public_pages USING btree (featured) WHERE (featured = true);

CREATE INDEX IF NOT EXISTS idx_public_pages_status ON public.public_pages USING btree (status);

CREATE INDEX IF NOT EXISTS idx_public_themes_directory_id ON public.public_themes USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_referrals_code ON public.referrals USING btree (referral_code);

CREATE INDEX IF NOT EXISTS idx_referrals_referrer ON public.referrals USING btree (referrer_id, referrer_type);

CREATE INDEX IF NOT EXISTS idx_referrals_status ON public.referrals USING btree (status);

CREATE INDEX IF NOT EXISTS idx_reviews_business ON public.reviews USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_reviews_business_id ON public.reviews USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_reviews_directory ON public.reviews USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_reviews_directory_id ON public.reviews USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_reviews_featured ON public.reviews USING btree (featured) WHERE (featured = true);

CREATE INDEX IF NOT EXISTS idx_reviews_status ON public.reviews USING btree (status);

CREATE INDEX IF NOT EXISTS idx_rfq_bids_bidder ON public.rfq_bids USING btree (bidder_business_id);

CREATE INDEX IF NOT EXISTS idx_rfq_bids_bidder_business_id ON public.rfq_bids USING btree (bidder_business_id);

CREATE INDEX IF NOT EXISTS idx_rfq_bids_rfq ON public.rfq_bids USING btree (rfq_id);

CREATE INDEX IF NOT EXISTS idx_rfq_bids_rfq_id ON public.rfq_bids USING btree (rfq_id);

CREATE INDEX IF NOT EXISTS idx_rfq_bids_status ON public.rfq_bids USING btree (status);

CREATE INDEX IF NOT EXISTS idx_rfq_messages_rfq ON public.rfq_messages USING btree (rfq_id);

CREATE INDEX IF NOT EXISTS idx_rfq_messages_rfq_id ON public.rfq_messages USING btree (rfq_id);

CREATE INDEX IF NOT EXISTS idx_rfq_messages_sender ON public.rfq_messages USING btree (sender_business_id);

CREATE INDEX IF NOT EXISTS idx_rfq_messages_sender_business_id ON public.rfq_messages USING btree (sender_business_id);

CREATE INDEX IF NOT EXISTS idx_rfqs_awarded_to ON public.rfqs USING btree (awarded_to);

CREATE INDEX IF NOT EXISTS idx_rfqs_category ON public.rfqs USING btree (category);

CREATE INDEX IF NOT EXISTS idx_rfqs_deadline ON public.rfqs USING btree (deadline) WHERE (status = 'open'::text);

CREATE INDEX IF NOT EXISTS idx_rfqs_poster ON public.rfqs USING btree (poster_business_id);

CREATE INDEX IF NOT EXISTS idx_rfqs_poster_business_id ON public.rfqs USING btree (poster_business_id);

CREATE INDEX IF NOT EXISTS idx_rfqs_status ON public.rfqs USING btree (status);

CREATE INDEX IF NOT EXISTS idx_schema_config_dir ON public.schema_config USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_schema_config_directory_id ON public.schema_config USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_search_config_directory_id ON public.search_config USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_seo_fallback_dir ON public.seo_fallback_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_seo_fallback_templates_directory_id ON public.seo_fallback_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_seo_meta_page ON public.seo_meta USING btree (page_type, page_id);

CREATE INDEX IF NOT EXISTS idx_service_bookings_business ON public.service_bookings USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_service_bookings_business_id ON public.service_bookings USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_service_bookings_directory_id ON public.service_bookings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_service_bookings_status ON public.service_bookings USING btree (status);

CREATE INDEX IF NOT EXISTS idx_service_bookings_visitor ON public.service_bookings USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_service_bookings_visitor_account_id ON public.service_bookings USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_service_prices_directory_id ON public.service_prices USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_service_prices_network_id ON public.service_prices USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_settlement_invoices_business_id ON public.settlement_invoices USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_settlement_invoices_run_id ON public.settlement_invoices USING btree (run_id);

CREATE INDEX IF NOT EXISTS idx_settlement_invoices_status ON public.settlement_invoices USING btree (status);

CREATE INDEX IF NOT EXISTS idx_settlement_payouts_business_id ON public.settlement_payouts USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_settlement_payouts_run_id ON public.settlement_payouts USING btree (run_id);

CREATE INDEX IF NOT EXISTS idx_settlement_payouts_status ON public.settlement_payouts USING btree (status);

CREATE INDEX IF NOT EXISTS idx_settlement_runs_created_by ON public.settlement_runs USING btree (created_by);

CREATE INDEX IF NOT EXISTS idx_settlement_runs_network_id ON public.settlement_runs USING btree (network_id);

CREATE INDEX IF NOT EXISTS idx_settlement_runs_period_key ON public.settlement_runs USING btree (period_key);

CREATE INDEX IF NOT EXISTS idx_settlement_runs_status ON public.settlement_runs USING btree (status);

CREATE INDEX IF NOT EXISTS idx_shared_leads_category ON public.shared_leads USING btree (category);

CREATE INDEX IF NOT EXISTS idx_shared_leads_claimed_by ON public.shared_leads USING btree (claimed_by);

CREATE INDEX IF NOT EXISTS idx_shared_leads_poster ON public.shared_leads USING btree (poster_business_id);

CREATE INDEX IF NOT EXISTS idx_shared_leads_poster_business_id ON public.shared_leads USING btree (poster_business_id);

CREATE INDEX IF NOT EXISTS idx_shared_leads_status ON public.shared_leads USING btree (status);

CREATE INDEX IF NOT EXISTS idx_sitemap_config_dir ON public.sitemap_config USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_sitemap_config_directory_id ON public.sitemap_config USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_slot_bookings_active ON public.plan_slot_bookings USING btree (city_slug, plan_tier_id, status) WHERE (status = 'active'::text);

CREATE INDEX IF NOT EXISTS idx_slot_bookings_city_plan ON public.plan_slot_bookings USING btree (city_slug, plan_tier_id);

CREATE INDEX IF NOT EXISTS idx_slot_bookings_date ON public.plan_slot_bookings USING btree (start_date, end_date);

CREATE INDEX IF NOT EXISTS idx_sponsored_listings_active ON public.sponsored_listings USING btree (directory_id, is_active, end_date);

CREATE INDEX IF NOT EXISTS idx_sponsored_listings_business ON public.sponsored_listings USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_sponsored_listings_business_id ON public.sponsored_listings USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_sponsored_listings_directory ON public.sponsored_listings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_sponsored_listings_directory_id ON public.sponsored_listings USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_sponsors_business_id ON public.sponsors USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_sponsors_directory_id ON public.sponsors USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_submissions_directory_id ON public.submissions USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_supplier_products_active ON public.supplier_products USING btree (is_active);

CREATE INDEX IF NOT EXISTS idx_supplier_products_business ON public.supplier_products USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_supplier_products_business_id ON public.supplier_products USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_supplier_products_category ON public.supplier_products USING btree (category);

CREATE INDEX IF NOT EXISTS idx_survey_responses_audience ON public.survey_responses USING btree (directory_id, audience);

CREATE INDEX IF NOT EXISTS idx_survey_responses_completed ON public.survey_responses USING btree (directory_id, completed_at DESC);

CREATE INDEX IF NOT EXISTS idx_survey_responses_directory ON public.survey_responses USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_survey_responses_directory_id ON public.survey_responses USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_survey_responses_survey ON public.survey_responses USING btree (survey_id);

CREATE INDEX IF NOT EXISTS idx_survey_responses_survey_id ON public.survey_responses USING btree (survey_id);

CREATE INDEX IF NOT EXISTS idx_survey_responses_visitor ON public.survey_responses USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_survey_responses_visitor_account_id ON public.survey_responses USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_tag_rules_tag ON public.tag_rules USING btree (tag_id);

CREATE INDEX IF NOT EXISTS idx_tag_rules_tenant ON public.tag_rules USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_tag_rules_tenant_id ON public.tag_rules USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_template_categories_active ON public.template_categories USING btree (sort_order) WHERE is_active;

CREATE INDEX IF NOT EXISTS idx_topic_format_templates_directory_id ON public.topic_format_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_tracked_links_short_code ON public.tracked_links USING btree (short_code);

CREATE INDEX IF NOT EXISTS idx_tracked_links_tenant ON public.tracked_links USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_tracked_links_tenant_id ON public.tracked_links USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_trap_door_templates_active ON public.trap_door_templates USING btree (is_active);

CREATE INDEX IF NOT EXISTS idx_trap_door_templates_directory ON public.trap_door_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_trap_door_templates_directory_id ON public.trap_door_templates USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_twilio_numbers_directory_id ON public.twilio_numbers USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_users_email ON public.users USING btree (email);

CREATE INDEX IF NOT EXISTS idx_users_tenant ON public.users USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_users_tenant_id ON public.users USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_visitor_accounts_coreswift_contact ON public.visitor_accounts USING btree (coreswift_contact_id) WHERE (coreswift_contact_id IS NOT NULL);

CREATE INDEX IF NOT EXISTS idx_visitor_accounts_directory ON public.visitor_accounts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_accounts_directory_id ON public.visitor_accounts USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_accounts_email ON public.visitor_accounts USING btree (email);

CREATE INDEX IF NOT EXISTS idx_visitor_events_business ON public.visitor_events USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_business_id ON public.visitor_events USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_category ON public.visitor_events USING btree (category_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_created ON public.visitor_events USING btree (created_at);

CREATE INDEX IF NOT EXISTS idx_visitor_events_directory ON public.visitor_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_directory_id ON public.visitor_events USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_session ON public.visitor_events USING btree (session_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_session_id ON public.visitor_events USING btree (session_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_type ON public.visitor_events USING btree (event_type);

CREATE INDEX IF NOT EXISTS idx_visitor_events_visitor ON public.visitor_events USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_visitor_events_visitor_id ON public.visitor_events USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_visitor_favorites_business_id ON public.visitor_favorites USING btree (business_id);

CREATE INDEX IF NOT EXISTS idx_visitor_favorites_directory ON public.visitor_favorites USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_favorites_directory_id ON public.visitor_favorites USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_favorites_visitor ON public.visitor_favorites USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_visitor_favorites_visitor_account_id ON public.visitor_favorites USING btree (visitor_account_id);

CREATE INDEX IF NOT EXISTS idx_visitor_sessions_directory ON public.visitor_sessions USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_sessions_directory_id ON public.visitor_sessions USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_visitor_sessions_started ON public.visitor_sessions USING btree (started_at);

CREATE INDEX IF NOT EXISTS idx_visitor_sessions_visitor ON public.visitor_sessions USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_visitor_sessions_visitor_id ON public.visitor_sessions USING btree (visitor_id);

CREATE INDEX IF NOT EXISTS idx_visitors_fingerprint ON public.visitors USING btree (fingerprint);

CREATE INDEX IF NOT EXISTS idx_visitors_ip ON public.visitors USING btree (ip_address);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_status ON public.webhook_deliveries USING btree (status);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_webhook ON public.webhook_deliveries USING btree (webhook_id);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_webhook_id ON public.webhook_deliveries USING btree (webhook_id);

CREATE INDEX IF NOT EXISTS idx_webhooks_directory ON public.webhooks USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_webhooks_directory_id ON public.webhooks USING btree (directory_id);

CREATE INDEX IF NOT EXISTS idx_webhooks_events ON public.webhooks USING gin (events);

CREATE INDEX IF NOT EXISTS idx_webhooks_tenant ON public.webhooks USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_webhooks_tenant_id ON public.webhooks USING btree (tenant_id);

CREATE INDEX IF NOT EXISTS idx_webhooks_user_id ON public.webhooks USING btree (user_id);

CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_directory ON public.provider_keys USING btree (directory_id, provider) WHERE ((is_default = true) AND (directory_id IS NOT NULL));

CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_network ON public.provider_keys USING btree (network_id, provider) WHERE ((is_default = true) AND (network_id IS NOT NULL));

CREATE UNIQUE INDEX IF NOT EXISTS provider_keys_one_default_per_provider_scope ON public.provider_keys USING btree (provider, COALESCE(scope, 'global'::character varying), COALESCE((network_id)::text, ''::text), COALESCE((directory_id)::text, ''::text)) WHERE is_default;

CREATE UNIQUE INDEX IF NOT EXISTS uq_content_research_topic_question ON public.content_research USING btree (topic_id, question);

CREATE UNIQUE INDEX IF NOT EXISTS uq_directory_surveys_directory_audience ON public.directory_surveys USING btree (directory_id, audience);

CREATE UNIQUE INDEX IF NOT EXISTS uq_loyalty_members_network_visitor ON public.loyalty_members USING btree (visitor_account_id) WHERE (network_id IS NOT NULL);

-- public.businesses.trg_auto_record_event
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_auto_record_event' AND tgrelid = 'public.businesses'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_auto_record_event AFTER INSERT OR DELETE OR UPDATE ON public.businesses FOR EACH ROW EXECUTE FUNCTION public.auto_record_directory_event();
    END IF;
END
$md_bl$;

-- public.directories.trg_auto_record_event
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_auto_record_event' AND tgrelid = 'public.directories'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_auto_record_event AFTER INSERT OR DELETE OR UPDATE ON public.directories FOR EACH ROW EXECUTE FUNCTION public.auto_record_directory_event();
    END IF;
END
$md_bl$;

-- public.business_services.trg_business_services_updated_at
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_business_services_updated_at' AND tgrelid = 'public.business_services'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_business_services_updated_at BEFORE UPDATE ON public.business_services FOR EACH ROW EXECUTE FUNCTION public.update_business_services_updated_at();
    END IF;
END
$md_bl$;

-- public.businesses.trg_businesses_search
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_businesses_search' AND tgrelid = 'public.businesses'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_businesses_search BEFORE INSERT OR UPDATE ON public.businesses FOR EACH ROW EXECUTE FUNCTION public.businesses_search_update();
    END IF;
END
$md_bl$;

-- public.referrals.trg_referrals_updated_at
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_referrals_updated_at' AND tgrelid = 'public.referrals'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_referrals_updated_at BEFORE UPDATE ON public.referrals FOR EACH ROW EXECUTE FUNCTION public.update_referrals_updated_at();
    END IF;
END
$md_bl$;

-- public.service_bookings.trg_service_bookings_updated_at
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_service_bookings_updated_at' AND tgrelid = 'public.service_bookings'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_service_bookings_updated_at BEFORE UPDATE ON public.service_bookings FOR EACH ROW EXECUTE FUNCTION public.update_service_bookings_updated_at();
    END IF;
END
$md_bl$;

-- public.businesses.trg_zn_businesses
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_zn_businesses' AND tgrelid = 'public.businesses'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_zn_businesses AFTER INSERT OR DELETE OR UPDATE ON public.businesses FOR EACH STATEMENT EXECUTE FUNCTION public.notify_zaarhub_change();
    END IF;
END
$md_bl$;

-- public.directory_categories.trg_zn_categories
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_zn_categories' AND tgrelid = 'public.directory_categories'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_zn_categories AFTER INSERT OR DELETE OR UPDATE ON public.directory_categories FOR EACH STATEMENT EXECUTE FUNCTION public.notify_zaarhub_change();
    END IF;
END
$md_bl$;

-- public.directories.trg_zn_directories
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_zn_directories' AND tgrelid = 'public.directories'::regclass AND NOT tgisinternal
    ) THEN
        CREATE TRIGGER trg_zn_directories AFTER INSERT OR DELETE OR UPDATE ON public.directories FOR EACH STATEMENT EXECUTE FUNCTION public.notify_zaarhub_change();
    END IF;
END
$md_bl$;

-- public._city_tags._city_tags_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = '_city_tags_directory_id_fkey' AND conrelid = 'public._city_tags'::regclass
    ) THEN
        ALTER TABLE public._city_tags ADD CONSTRAINT _city_tags_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.account_links.account_links_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'account_links_user_id_fkey' AND conrelid = 'public.account_links'::regclass
    ) THEN
        ALTER TABLE public.account_links ADD CONSTRAINT account_links_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.account_links.account_links_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'account_links_visitor_account_id_fkey' AND conrelid = 'public.account_links'::regclass
    ) THEN
        ALTER TABLE public.account_links ADD CONSTRAINT account_links_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_creatives.ad_creatives_sponsor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_creatives_sponsor_id_fkey' AND conrelid = 'public.ad_creatives'::regclass
    ) THEN
        ALTER TABLE public.ad_creatives ADD CONSTRAINT ad_creatives_sponsor_id_fkey FOREIGN KEY (sponsor_id) REFERENCES public.sponsors(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_earnings.ad_earnings_ad_zone_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_earnings_ad_zone_id_fkey' AND conrelid = 'public.ad_earnings'::regclass
    ) THEN
        ALTER TABLE public.ad_earnings ADD CONSTRAINT ad_earnings_ad_zone_id_fkey FOREIGN KEY (ad_zone_id) REFERENCES public.ad_zones(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_earnings.ad_earnings_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_earnings_directory_id_fkey' AND conrelid = 'public.ad_earnings'::regclass
    ) THEN
        ALTER TABLE public.ad_earnings ADD CONSTRAINT ad_earnings_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_earnings.ad_earnings_schedule_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_earnings_schedule_id_fkey' AND conrelid = 'public.ad_earnings'::regclass
    ) THEN
        ALTER TABLE public.ad_earnings ADD CONSTRAINT ad_earnings_schedule_id_fkey FOREIGN KEY (schedule_id) REFERENCES public.ad_schedules(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_earnings.ad_earnings_sponsor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_earnings_sponsor_id_fkey' AND conrelid = 'public.ad_earnings'::regclass
    ) THEN
        ALTER TABLE public.ad_earnings ADD CONSTRAINT ad_earnings_sponsor_id_fkey FOREIGN KEY (sponsor_id) REFERENCES public.sponsors(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_schedules.ad_schedules_ad_zone_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_schedules_ad_zone_id_fkey' AND conrelid = 'public.ad_schedules'::regclass
    ) THEN
        ALTER TABLE public.ad_schedules ADD CONSTRAINT ad_schedules_ad_zone_id_fkey FOREIGN KEY (ad_zone_id) REFERENCES public.ad_zones(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_schedules.ad_schedules_creative_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_schedules_creative_id_fkey' AND conrelid = 'public.ad_schedules'::regclass
    ) THEN
        ALTER TABLE public.ad_schedules ADD CONSTRAINT ad_schedules_creative_id_fkey FOREIGN KEY (creative_id) REFERENCES public.ad_creatives(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_schedules.ad_schedules_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_schedules_directory_id_fkey' AND conrelid = 'public.ad_schedules'::regclass
    ) THEN
        ALTER TABLE public.ad_schedules ADD CONSTRAINT ad_schedules_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_schedules.ad_schedules_sponsor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_schedules_sponsor_id_fkey' AND conrelid = 'public.ad_schedules'::regclass
    ) THEN
        ALTER TABLE public.ad_schedules ADD CONSTRAINT ad_schedules_sponsor_id_fkey FOREIGN KEY (sponsor_id) REFERENCES public.sponsors(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.ad_zones.ad_zones_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ad_zones_directory_id_fkey' AND conrelid = 'public.ad_zones'::regclass
    ) THEN
        ALTER TABLE public.ad_zones ADD CONSTRAINT ad_zones_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.analytics_events.analytics_events_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'analytics_events_directory_id_fkey' AND conrelid = 'public.analytics_events'::regclass
    ) THEN
        ALTER TABLE public.analytics_events ADD CONSTRAINT analytics_events_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.api_key_usage.api_key_usage_api_key_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'api_key_usage_api_key_id_fkey' AND conrelid = 'public.api_key_usage'::regclass
    ) THEN
        ALTER TABLE public.api_key_usage ADD CONSTRAINT api_key_usage_api_key_id_fkey FOREIGN KEY (api_key_id) REFERENCES public.api_keys(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.api_keys.api_keys_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'api_keys_tenant_id_fkey' AND conrelid = 'public.api_keys'::regclass
    ) THEN
        ALTER TABLE public.api_keys ADD CONSTRAINT api_keys_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);
    END IF;
END
$md_bl$;

-- public.api_keys.api_keys_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'api_keys_user_id_fkey' AND conrelid = 'public.api_keys'::regclass
    ) THEN
        ALTER TABLE public.api_keys ADD CONSTRAINT api_keys_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.approval_queue.approval_queue_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'approval_queue_directory_id_fkey' AND conrelid = 'public.approval_queue'::regclass
    ) THEN
        ALTER TABLE public.approval_queue ADD CONSTRAINT approval_queue_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.author_profiles.author_profiles_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'author_profiles_directory_id_fkey' AND conrelid = 'public.author_profiles'::regclass
    ) THEN
        ALTER TABLE public.author_profiles ADD CONSTRAINT author_profiles_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.author_profiles.author_profiles_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'author_profiles_user_id_fkey' AND conrelid = 'public.author_profiles'::regclass
    ) THEN
        ALTER TABLE public.author_profiles ADD CONSTRAINT author_profiles_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.b2b_notifications.b2b_notifications_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_notifications_business_id_fkey' AND conrelid = 'public.b2b_notifications'::regclass
    ) THEN
        ALTER TABLE public.b2b_notifications ADD CONSTRAINT b2b_notifications_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.b2b_notifications.b2b_notifications_related_message_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_notifications_related_message_id_fkey' AND conrelid = 'public.b2b_notifications'::regclass
    ) THEN
        ALTER TABLE public.b2b_notifications ADD CONSTRAINT b2b_notifications_related_message_id_fkey FOREIGN KEY (related_message_id) REFERENCES public.business_messages(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.b2b_notifications.b2b_notifications_related_order_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_notifications_related_order_id_fkey' AND conrelid = 'public.b2b_notifications'::regclass
    ) THEN
        ALTER TABLE public.b2b_notifications ADD CONSTRAINT b2b_notifications_related_order_id_fkey FOREIGN KEY (related_order_id) REFERENCES public.b2b_orders(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.b2b_orders.b2b_orders_buyer_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_orders_buyer_business_id_fkey' AND conrelid = 'public.b2b_orders'::regclass
    ) THEN
        ALTER TABLE public.b2b_orders ADD CONSTRAINT b2b_orders_buyer_business_id_fkey FOREIGN KEY (buyer_business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.b2b_orders.b2b_orders_product_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_orders_product_id_fkey' AND conrelid = 'public.b2b_orders'::regclass
    ) THEN
        ALTER TABLE public.b2b_orders ADD CONSTRAINT b2b_orders_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.supplier_products(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.b2b_orders.b2b_orders_supplier_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'b2b_orders_supplier_business_id_fkey' AND conrelid = 'public.b2b_orders'::regclass
    ) THEN
        ALTER TABLE public.b2b_orders ADD CONSTRAINT b2b_orders_supplier_business_id_fkey FOREIGN KEY (supplier_business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_media.blog_media_blog_post_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_media_blog_post_id_fkey' AND conrelid = 'public.blog_media'::regclass
    ) THEN
        ALTER TABLE public.blog_media ADD CONSTRAINT blog_media_blog_post_id_fkey FOREIGN KEY (blog_post_id) REFERENCES public.blog_posts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_author_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_author_id_fkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_author_id_fkey FOREIGN KEY (author_id) REFERENCES public.author_profiles(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_directory_id_fkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_location_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_location_id_fkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_location_id_fkey FOREIGN KEY (location_id) REFERENCES public.directory_locations(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_master_post_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_master_post_id_fkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_master_post_id_fkey FOREIGN KEY (master_post_id) REFERENCES public.blog_posts(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_service_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_service_id_fkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.directory_services(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.blog_posts.blog_posts_template_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_posts_template_id_fkey' AND conrelid = 'public.blog_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_posts ADD CONSTRAINT blog_posts_template_id_fkey FOREIGN KEY (template_id) REFERENCES public.blog_templates(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.blog_qa_keywords.blog_qa_keywords_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_qa_keywords_directory_id_fkey' AND conrelid = 'public.blog_qa_keywords'::regclass
    ) THEN
        ALTER TABLE public.blog_qa_keywords ADD CONSTRAINT blog_qa_keywords_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_qa_posts.blog_qa_posts_blog_post_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_qa_posts_blog_post_id_fkey' AND conrelid = 'public.blog_qa_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_qa_posts ADD CONSTRAINT blog_qa_posts_blog_post_id_fkey FOREIGN KEY (blog_post_id) REFERENCES public.blog_posts(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.blog_qa_posts.blog_qa_posts_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_qa_posts_directory_id_fkey' AND conrelid = 'public.blog_qa_posts'::regclass
    ) THEN
        ALTER TABLE public.blog_qa_posts ADD CONSTRAINT blog_qa_posts_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_template_directories.blog_template_directories_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_template_directories_directory_id_fkey' AND conrelid = 'public.blog_template_directories'::regclass
    ) THEN
        ALTER TABLE public.blog_template_directories ADD CONSTRAINT blog_template_directories_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_template_directories.blog_template_directories_template_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_template_directories_template_id_fkey' AND conrelid = 'public.blog_template_directories'::regclass
    ) THEN
        ALTER TABLE public.blog_template_directories ADD CONSTRAINT blog_template_directories_template_id_fkey FOREIGN KEY (template_id) REFERENCES public.blog_templates(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.blog_templates.blog_templates_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'blog_templates_directory_id_fkey' AND conrelid = 'public.blog_templates'::regclass
    ) THEN
        ALTER TABLE public.blog_templates ADD CONSTRAINT blog_templates_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.bundle_services.bundle_services_bundle_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'bundle_services_bundle_id_fkey' AND conrelid = 'public.bundle_services'::regclass
    ) THEN
        ALTER TABLE public.bundle_services ADD CONSTRAINT bundle_services_bundle_id_fkey FOREIGN KEY (bundle_id) REFERENCES public.price_bundles(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_articles.business_articles_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_articles_business_id_fkey' AND conrelid = 'public.business_articles'::regclass
    ) THEN
        ALTER TABLE public.business_articles ADD CONSTRAINT business_articles_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.business_articles.business_articles_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_articles_directory_id_fkey' AND conrelid = 'public.business_articles'::regclass
    ) THEN
        ALTER TABLE public.business_articles ADD CONSTRAINT business_articles_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_categories.business_categories_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_categories_business_id_fkey' AND conrelid = 'public.business_categories'::regclass
    ) THEN
        ALTER TABLE public.business_categories ADD CONSTRAINT business_categories_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_categories.business_categories_category_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_categories_category_id_fkey' AND conrelid = 'public.business_categories'::regclass
    ) THEN
        ALTER TABLE public.business_categories ADD CONSTRAINT business_categories_category_id_fkey FOREIGN KEY (category_id) REFERENCES public.directory_categories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_listings.business_listings_city_page_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_listings_city_page_id_fkey' AND conrelid = 'public.business_listings'::regclass
    ) THEN
        ALTER TABLE public.business_listings ADD CONSTRAINT business_listings_city_page_id_fkey FOREIGN KEY (city_page_id) REFERENCES public.city_pages(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_messages.business_messages_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_messages_business_id_fkey' AND conrelid = 'public.business_messages'::regclass
    ) THEN
        ALTER TABLE public.business_messages ADD CONSTRAINT business_messages_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_messages.business_messages_sender_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_messages_sender_business_id_fkey' AND conrelid = 'public.business_messages'::regclass
    ) THEN
        ALTER TABLE public.business_messages ADD CONSTRAINT business_messages_sender_business_id_fkey FOREIGN KEY (sender_business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.business_messages.business_messages_to_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_messages_to_business_id_fkey' AND conrelid = 'public.business_messages'::regclass
    ) THEN
        ALTER TABLE public.business_messages ADD CONSTRAINT business_messages_to_business_id_fkey FOREIGN KEY (to_business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_meta.business_meta_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_meta_business_id_fkey' AND conrelid = 'public.business_meta'::regclass
    ) THEN
        ALTER TABLE public.business_meta ADD CONSTRAINT business_meta_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_point_ledger.business_point_ledger_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_point_ledger_network_id_fkey' AND conrelid = 'public.business_point_ledger'::regclass
    ) THEN
        ALTER TABLE public.business_point_ledger ADD CONSTRAINT business_point_ledger_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_services.business_services_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_services_business_id_fkey' AND conrelid = 'public.business_services'::regclass
    ) THEN
        ALTER TABLE public.business_services ADD CONSTRAINT business_services_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_services.business_services_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_services_directory_id_fkey' AND conrelid = 'public.business_services'::regclass
    ) THEN
        ALTER TABLE public.business_services ADD CONSTRAINT business_services_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_subscriptions.business_subscriptions_tier_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_subscriptions_tier_id_fkey' AND conrelid = 'public.business_subscriptions'::regclass
    ) THEN
        ALTER TABLE public.business_subscriptions ADD CONSTRAINT business_subscriptions_tier_id_fkey FOREIGN KEY (tier_id) REFERENCES public.plan_tiers(id);
    END IF;
END
$md_bl$;

-- public.business_transfer_events.business_transfer_events_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfer_events_business_id_fkey' AND conrelid = 'public.business_transfer_events'::regclass
    ) THEN
        ALTER TABLE public.business_transfer_events ADD CONSTRAINT business_transfer_events_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.business_transfer_events.business_transfer_events_transfer_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfer_events_transfer_id_fkey' AND conrelid = 'public.business_transfer_events'::regclass
    ) THEN
        ALTER TABLE public.business_transfer_events ADD CONSTRAINT business_transfer_events_transfer_id_fkey FOREIGN KEY (transfer_id) REFERENCES public.business_transfers(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_transfer_fees.business_transfer_fees_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfer_fees_business_id_fkey' AND conrelid = 'public.business_transfer_fees'::regclass
    ) THEN
        ALTER TABLE public.business_transfer_fees ADD CONSTRAINT business_transfer_fees_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.business_transfer_fees.business_transfer_fees_transfer_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfer_fees_transfer_id_fkey' AND conrelid = 'public.business_transfer_fees'::regclass
    ) THEN
        ALTER TABLE public.business_transfer_fees ADD CONSTRAINT business_transfer_fees_transfer_id_fkey FOREIGN KEY (transfer_id) REFERENCES public.business_transfers(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_transfers.business_transfers_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfers_business_id_fkey' AND conrelid = 'public.business_transfers'::regclass
    ) THEN
        ALTER TABLE public.business_transfers ADD CONSTRAINT business_transfers_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_transfers.business_transfers_target_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_transfers_target_directory_id_fkey' AND conrelid = 'public.business_transfers'::regclass
    ) THEN
        ALTER TABLE public.business_transfers ADD CONSTRAINT business_transfers_target_directory_id_fkey FOREIGN KEY (target_directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.business_verifications.business_verifications_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_verifications_business_id_fkey' AND conrelid = 'public.business_verifications'::regclass
    ) THEN
        ALTER TABLE public.business_verifications ADD CONSTRAINT business_verifications_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.business_verifications.business_verifications_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_verifications_directory_id_fkey' AND conrelid = 'public.business_verifications'::regclass
    ) THEN
        ALTER TABLE public.business_verifications ADD CONSTRAINT business_verifications_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.business_verifications.business_verifications_verified_by_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'business_verifications_verified_by_fkey' AND conrelid = 'public.business_verifications'::regclass
    ) THEN
        ALTER TABLE public.business_verifications ADD CONSTRAINT business_verifications_verified_by_fkey FOREIGN KEY (verified_by) REFERENCES public.users(id);
    END IF;
END
$md_bl$;

-- public.businesses.businesses_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'businesses_directory_id_fkey' AND conrelid = 'public.businesses'::regclass
    ) THEN
        ALTER TABLE public.businesses ADD CONSTRAINT businesses_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.businesses.businesses_featured_deal_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'businesses_featured_deal_id_fkey' AND conrelid = 'public.businesses'::regclass
    ) THEN
        ALTER TABLE public.businesses ADD CONSTRAINT businesses_featured_deal_id_fkey FOREIGN KEY (featured_deal_id) REFERENCES public.deals(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.businesses.businesses_featured_product_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'businesses_featured_product_id_fkey' AND conrelid = 'public.businesses'::regclass
    ) THEN
        ALTER TABLE public.businesses ADD CONSTRAINT businesses_featured_product_id_fkey FOREIGN KEY (featured_product_id) REFERENCES public.supplier_products(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.buying_group_deals.buying_group_deals_group_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_deals_group_id_fkey' AND conrelid = 'public.buying_group_deals'::regclass
    ) THEN
        ALTER TABLE public.buying_group_deals ADD CONSTRAINT buying_group_deals_group_id_fkey FOREIGN KEY (group_id) REFERENCES public.buying_groups(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.buying_group_deals.buying_group_deals_supplier_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_deals_supplier_business_id_fkey' AND conrelid = 'public.buying_group_deals'::regclass
    ) THEN
        ALTER TABLE public.buying_group_deals ADD CONSTRAINT buying_group_deals_supplier_business_id_fkey FOREIGN KEY (supplier_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.buying_group_members.buying_group_members_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_members_business_id_fkey' AND conrelid = 'public.buying_group_members'::regclass
    ) THEN
        ALTER TABLE public.buying_group_members ADD CONSTRAINT buying_group_members_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.buying_group_members.buying_group_members_group_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_group_members_group_id_fkey' AND conrelid = 'public.buying_group_members'::regclass
    ) THEN
        ALTER TABLE public.buying_group_members ADD CONSTRAINT buying_group_members_group_id_fkey FOREIGN KEY (group_id) REFERENCES public.buying_groups(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.buying_groups.buying_groups_founder_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'buying_groups_founder_business_id_fkey' AND conrelid = 'public.buying_groups'::regclass
    ) THEN
        ALTER TABLE public.buying_groups ADD CONSTRAINT buying_groups_founder_business_id_fkey FOREIGN KEY (founder_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.call_logs.call_logs_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'call_logs_directory_id_fkey' AND conrelid = 'public.call_logs'::regclass
    ) THEN
        ALTER TABLE public.call_logs ADD CONSTRAINT call_logs_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.category_redeem_caps.category_redeem_caps_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'category_redeem_caps_network_id_fkey' AND conrelid = 'public.category_redeem_caps'::regclass
    ) THEN
        ALTER TABLE public.category_redeem_caps ADD CONSTRAINT category_redeem_caps_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.category_requests.category_requests_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'category_requests_business_id_fkey' AND conrelid = 'public.category_requests'::regclass
    ) THEN
        ALTER TABLE public.category_requests ADD CONSTRAINT category_requests_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.category_requests.category_requests_category_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'category_requests_category_id_fkey' AND conrelid = 'public.category_requests'::regclass
    ) THEN
        ALTER TABLE public.category_requests ADD CONSTRAINT category_requests_category_id_fkey FOREIGN KEY (category_id) REFERENCES public.directory_categories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.checkout_sessions.checkout_sessions_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'checkout_sessions_business_id_fkey' AND conrelid = 'public.checkout_sessions'::regclass
    ) THEN
        ALTER TABLE public.checkout_sessions ADD CONSTRAINT checkout_sessions_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.checkout_sessions.checkout_sessions_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'checkout_sessions_directory_id_fkey' AND conrelid = 'public.checkout_sessions'::regclass
    ) THEN
        ALTER TABLE public.checkout_sessions ADD CONSTRAINT checkout_sessions_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.city_plan_slots.city_plan_slots_plan_tier_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_plan_slots_plan_tier_id_fkey' AND conrelid = 'public.city_plan_slots'::regclass
    ) THEN
        ALTER TABLE public.city_plan_slots ADD CONSTRAINT city_plan_slots_plan_tier_id_fkey FOREIGN KEY (plan_tier_id) REFERENCES public.plan_tiers(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.city_requests.city_requests_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'city_requests_directory_id_fkey' AND conrelid = 'public.city_requests'::regclass
    ) THEN
        ALTER TABLE public.city_requests ADD CONSTRAINT city_requests_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.claim_offers.claim_offers_listing_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claim_offers_listing_id_fkey' AND conrelid = 'public.claim_offers'::regclass
    ) THEN
        ALTER TABLE public.claim_offers ADD CONSTRAINT claim_offers_listing_id_fkey FOREIGN KEY (listing_id) REFERENCES public.business_listings(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.claimed_businesses.claimed_businesses_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claimed_businesses_business_id_fkey' AND conrelid = 'public.claimed_businesses'::regclass
    ) THEN
        ALTER TABLE public.claimed_businesses ADD CONSTRAINT claimed_businesses_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.claimed_businesses.claimed_businesses_subscription_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claimed_businesses_subscription_id_fkey' AND conrelid = 'public.claimed_businesses'::regclass
    ) THEN
        ALTER TABLE public.claimed_businesses ADD CONSTRAINT claimed_businesses_subscription_id_fkey FOREIGN KEY (subscription_id) REFERENCES public.business_subscriptions(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.claimed_businesses.claimed_businesses_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claimed_businesses_user_id_fkey' AND conrelid = 'public.claimed_businesses'::regclass
    ) THEN
        ALTER TABLE public.claimed_businesses ADD CONSTRAINT claimed_businesses_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.claimed_businesses.claimed_businesses_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'claimed_businesses_visitor_account_id_fkey' AND conrelid = 'public.claimed_businesses'::regclass
    ) THEN
        ALTER TABLE public.claimed_businesses ADD CONSTRAINT claimed_businesses_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.community_events.community_events_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'community_events_business_id_fkey' AND conrelid = 'public.community_events'::regclass
    ) THEN
        ALTER TABLE public.community_events ADD CONSTRAINT community_events_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.community_events.community_events_created_by_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'community_events_created_by_fkey' AND conrelid = 'public.community_events'::regclass
    ) THEN
        ALTER TABLE public.community_events ADD CONSTRAINT community_events_created_by_fkey FOREIGN KEY (created_by) REFERENCES public.users(id);
    END IF;
END
$md_bl$;

-- public.community_events.community_events_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'community_events_directory_id_fkey' AND conrelid = 'public.community_events'::regclass
    ) THEN
        ALTER TABLE public.community_events ADD CONSTRAINT community_events_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.community_events.community_events_loyalty_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'community_events_loyalty_program_id_fkey' AND conrelid = 'public.community_events'::regclass
    ) THEN
        ALTER TABLE public.community_events ADD CONSTRAINT community_events_loyalty_program_id_fkey FOREIGN KEY (loyalty_program_id) REFERENCES public.loyalty_programs(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.community_events.community_events_source_provider_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'community_events_source_provider_id_fkey' AND conrelid = 'public.community_events'::regclass
    ) THEN
        ALTER TABLE public.community_events ADD CONSTRAINT community_events_source_provider_id_fkey FOREIGN KEY (source_provider_id) REFERENCES public.event_providers(id);
    END IF;
END
$md_bl$;

-- public.connected_services.connected_services_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'connected_services_user_id_fkey' AND conrelid = 'public.connected_services'::regclass
    ) THEN
        ALTER TABLE public.connected_services ADD CONSTRAINT connected_services_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.content_queue.content_queue_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_queue_directory_id_fkey' AND conrelid = 'public.content_queue'::regclass
    ) THEN
        ALTER TABLE public.content_queue ADD CONSTRAINT content_queue_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.content_research.content_research_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_research_directory_id_fkey' AND conrelid = 'public.content_research'::regclass
    ) THEN
        ALTER TABLE public.content_research ADD CONSTRAINT content_research_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.content_research.content_research_topic_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_research_topic_id_fkey' AND conrelid = 'public.content_research'::regclass
    ) THEN
        ALTER TABLE public.content_research ADD CONSTRAINT content_research_topic_id_fkey FOREIGN KEY (topic_id) REFERENCES public.content_topics(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.content_topics.content_topics_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_topics_directory_id_fkey' AND conrelid = 'public.content_topics'::regclass
    ) THEN
        ALTER TABLE public.content_topics ADD CONSTRAINT content_topics_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.content_topics.content_topics_location_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_topics_location_id_fkey' AND conrelid = 'public.content_topics'::regclass
    ) THEN
        ALTER TABLE public.content_topics ADD CONSTRAINT content_topics_location_id_fkey FOREIGN KEY (location_id) REFERENCES public.directory_locations(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.content_topics.content_topics_service_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'content_topics_service_id_fkey' AND conrelid = 'public.content_topics'::regclass
    ) THEN
        ALTER TABLE public.content_topics ADD CONSTRAINT content_topics_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.directory_services(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.crm_contacts.crm_contacts_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_contacts_directory_id_fkey' AND conrelid = 'public.crm_contacts'::regclass
    ) THEN
        ALTER TABLE public.crm_contacts ADD CONSTRAINT crm_contacts_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.crm_deal_records.crm_deal_records_contact_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_deal_records_contact_id_fkey' AND conrelid = 'public.crm_deal_records'::regclass
    ) THEN
        ALTER TABLE public.crm_deal_records ADD CONSTRAINT crm_deal_records_contact_id_fkey FOREIGN KEY (contact_id) REFERENCES public.crm_contacts(id);
    END IF;
END
$md_bl$;

-- public.crm_deal_records.crm_deal_records_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_deal_records_directory_id_fkey' AND conrelid = 'public.crm_deal_records'::regclass
    ) THEN
        ALTER TABLE public.crm_deal_records ADD CONSTRAINT crm_deal_records_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.crm_deal_records.crm_deal_records_pipeline_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_deal_records_pipeline_id_fkey' AND conrelid = 'public.crm_deal_records'::regclass
    ) THEN
        ALTER TABLE public.crm_deal_records ADD CONSTRAINT crm_deal_records_pipeline_id_fkey FOREIGN KEY (pipeline_id) REFERENCES public.crm_pipelines(id);
    END IF;
END
$md_bl$;

-- public.crm_pipelines.crm_pipelines_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'crm_pipelines_directory_id_fkey' AND conrelid = 'public.crm_pipelines'::regclass
    ) THEN
        ALTER TABLE public.crm_pipelines ADD CONSTRAINT crm_pipelines_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.data_enrichment_logs.data_enrichment_logs_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'data_enrichment_logs_business_id_fkey' AND conrelid = 'public.data_enrichment_logs'::regclass
    ) THEN
        ALTER TABLE public.data_enrichment_logs ADD CONSTRAINT data_enrichment_logs_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.data_enrichment_logs.data_enrichment_logs_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'data_enrichment_logs_directory_id_fkey' AND conrelid = 'public.data_enrichment_logs'::regclass
    ) THEN
        ALTER TABLE public.data_enrichment_logs ADD CONSTRAINT data_enrichment_logs_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.deal_claims.deal_claims_deal_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_claims_deal_id_fkey' AND conrelid = 'public.deal_claims'::regclass
    ) THEN
        ALTER TABLE public.deal_claims ADD CONSTRAINT deal_claims_deal_id_fkey FOREIGN KEY (deal_id) REFERENCES public.deals(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.deal_redemptions.deal_redemptions_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_redemptions_business_id_fkey' AND conrelid = 'public.deal_redemptions'::regclass
    ) THEN
        ALTER TABLE public.deal_redemptions ADD CONSTRAINT deal_redemptions_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.deal_redemptions.deal_redemptions_deal_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_redemptions_deal_id_fkey' AND conrelid = 'public.deal_redemptions'::regclass
    ) THEN
        ALTER TABLE public.deal_redemptions ADD CONSTRAINT deal_redemptions_deal_id_fkey FOREIGN KEY (deal_id) REFERENCES public.deals(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.deal_redemptions.deal_redemptions_visitor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deal_redemptions_visitor_id_fkey' AND conrelid = 'public.deal_redemptions'::regclass
    ) THEN
        ALTER TABLE public.deal_redemptions ADD CONSTRAINT deal_redemptions_visitor_id_fkey FOREIGN KEY (visitor_id) REFERENCES public.visitor_accounts(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.deals.deals_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deals_directory_id_fkey' AND conrelid = 'public.deals'::regclass
    ) THEN
        ALTER TABLE public.deals ADD CONSTRAINT deals_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.deals.deals_loyalty_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'deals_loyalty_program_id_fkey' AND conrelid = 'public.deals'::regclass
    ) THEN
        ALTER TABLE public.deals ADD CONSTRAINT deals_loyalty_program_id_fkey FOREIGN KEY (loyalty_program_id) REFERENCES public.loyalty_programs(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.directories.directories_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directories_network_id_fkey' AND conrelid = 'public.directories'::regclass
    ) THEN
        ALTER TABLE public.directories ADD CONSTRAINT directories_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.directory_branding.directory_branding_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_branding_directory_id_fkey' AND conrelid = 'public.directory_branding'::regclass
    ) THEN
        ALTER TABLE public.directory_branding ADD CONSTRAINT directory_branding_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_categories.directory_categories_parent_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_categories_parent_id_fkey' AND conrelid = 'public.directory_categories'::regclass
    ) THEN
        ALTER TABLE public.directory_categories ADD CONSTRAINT directory_categories_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.directory_categories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.directory_email_settings.directory_email_settings_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_email_settings_directory_id_fkey' AND conrelid = 'public.directory_email_settings'::regclass
    ) THEN
        ALTER TABLE public.directory_email_settings ADD CONSTRAINT directory_email_settings_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_events.directory_events_actor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_events_actor_id_fkey' AND conrelid = 'public.directory_events'::regclass
    ) THEN
        ALTER TABLE public.directory_events ADD CONSTRAINT directory_events_actor_id_fkey FOREIGN KEY (actor_id) REFERENCES public.users(id);
    END IF;
END
$md_bl$;

-- public.directory_events.directory_events_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_events_directory_id_fkey' AND conrelid = 'public.directory_events'::regclass
    ) THEN
        ALTER TABLE public.directory_events ADD CONSTRAINT directory_events_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.directory_events.directory_events_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_events_tenant_id_fkey' AND conrelid = 'public.directory_events'::regclass
    ) THEN
        ALTER TABLE public.directory_events ADD CONSTRAINT directory_events_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);
    END IF;
END
$md_bl$;

-- public.directory_locations.directory_locations_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_locations_directory_id_fkey' AND conrelid = 'public.directory_locations'::regclass
    ) THEN
        ALTER TABLE public.directory_locations ADD CONSTRAINT directory_locations_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_notifications.directory_notifications_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_notifications_directory_id_fkey' AND conrelid = 'public.directory_notifications'::regclass
    ) THEN
        ALTER TABLE public.directory_notifications ADD CONSTRAINT directory_notifications_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_services.directory_services_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_services_directory_id_fkey' AND conrelid = 'public.directory_services'::regclass
    ) THEN
        ALTER TABLE public.directory_services ADD CONSTRAINT directory_services_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_surveys.directory_surveys_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_surveys_directory_id_fkey' AND conrelid = 'public.directory_surveys'::regclass
    ) THEN
        ALTER TABLE public.directory_surveys ADD CONSTRAINT directory_surveys_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_surveys.directory_surveys_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_surveys_network_id_fkey' AND conrelid = 'public.directory_surveys'::regclass
    ) THEN
        ALTER TABLE public.directory_surveys ADD CONSTRAINT directory_surveys_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.directory_tiers.directory_tiers_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_tiers_directory_id_fkey' AND conrelid = 'public.directory_tiers'::regclass
    ) THEN
        ALTER TABLE public.directory_tiers ADD CONSTRAINT directory_tiers_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.directory_tiers.directory_tiers_plan_tier_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'directory_tiers_plan_tier_id_fkey' AND conrelid = 'public.directory_tiers'::regclass
    ) THEN
        ALTER TABLE public.directory_tiers ADD CONSTRAINT directory_tiers_plan_tier_id_fkey FOREIGN KEY (plan_tier_id) REFERENCES public.plan_tiers(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.discovery_queue.discovery_queue_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'discovery_queue_directory_id_fkey' AND conrelid = 'public.discovery_queue'::regclass
    ) THEN
        ALTER TABLE public.discovery_queue ADD CONSTRAINT discovery_queue_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.domain_mappings.domain_mappings_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'domain_mappings_directory_id_fkey' AND conrelid = 'public.domain_mappings'::regclass
    ) THEN
        ALTER TABLE public.domain_mappings ADD CONSTRAINT domain_mappings_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.email_campaigns.email_campaigns_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'email_campaigns_directory_id_fkey' AND conrelid = 'public.email_campaigns'::regclass
    ) THEN
        ALTER TABLE public.email_campaigns ADD CONSTRAINT email_campaigns_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.email_campaigns.email_campaigns_template_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'email_campaigns_template_id_fkey' AND conrelid = 'public.email_campaigns'::regclass
    ) THEN
        ALTER TABLE public.email_campaigns ADD CONSTRAINT email_campaigns_template_id_fkey FOREIGN KEY (template_id) REFERENCES public.email_templates(id);
    END IF;
END
$md_bl$;

-- public.email_templates.email_templates_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'email_templates_directory_id_fkey' AND conrelid = 'public.email_templates'::regclass
    ) THEN
        ALTER TABLE public.email_templates ADD CONSTRAINT email_templates_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.event_providers.event_providers_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'event_providers_directory_id_fkey' AND conrelid = 'public.event_providers'::regclass
    ) THEN
        ALTER TABLE public.event_providers ADD CONSTRAINT event_providers_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.event_rsvps.event_rsvps_event_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'event_rsvps_event_id_fkey' AND conrelid = 'public.event_rsvps'::regclass
    ) THEN
        ALTER TABLE public.event_rsvps ADD CONSTRAINT event_rsvps_event_id_fkey FOREIGN KEY (event_id) REFERENCES public.community_events(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.event_rsvps.event_rsvps_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'event_rsvps_visitor_account_id_fkey' AND conrelid = 'public.event_rsvps'::regclass
    ) THEN
        ALTER TABLE public.event_rsvps ADD CONSTRAINT event_rsvps_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.export_templates.export_templates_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'export_templates_directory_id_fkey' AND conrelid = 'public.export_templates'::regclass
    ) THEN
        ALTER TABLE public.export_templates ADD CONSTRAINT export_templates_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.grandfathered_pricing.grandfathered_pricing_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'grandfathered_pricing_business_id_fkey' AND conrelid = 'public.grandfathered_pricing'::regclass
    ) THEN
        ALTER TABLE public.grandfathered_pricing ADD CONSTRAINT grandfathered_pricing_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.group_deal_commitments.group_deal_commitments_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'group_deal_commitments_business_id_fkey' AND conrelid = 'public.group_deal_commitments'::regclass
    ) THEN
        ALTER TABLE public.group_deal_commitments ADD CONSTRAINT group_deal_commitments_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.group_deal_commitments.group_deal_commitments_deal_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'group_deal_commitments_deal_id_fkey' AND conrelid = 'public.group_deal_commitments'::regclass
    ) THEN
        ALTER TABLE public.group_deal_commitments ADD CONSTRAINT group_deal_commitments_deal_id_fkey FOREIGN KEY (deal_id) REFERENCES public.buying_group_deals(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.homepage_sections.homepage_sections_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_sections_directory_id_fkey' AND conrelid = 'public.homepage_sections'::regclass
    ) THEN
        ALTER TABLE public.homepage_sections ADD CONSTRAINT homepage_sections_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.homepage_sections.homepage_sections_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_sections_network_id_fkey' AND conrelid = 'public.homepage_sections'::regclass
    ) THEN
        ALTER TABLE public.homepage_sections ADD CONSTRAINT homepage_sections_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.import_logs.import_logs_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'import_logs_directory_id_fkey' AND conrelid = 'public.import_logs'::regclass
    ) THEN
        ALTER TABLE public.import_logs ADD CONSTRAINT import_logs_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.landing_pages.landing_pages_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'landing_pages_directory_id_fkey' AND conrelid = 'public.landing_pages'::regclass
    ) THEN
        ALTER TABLE public.landing_pages ADD CONSTRAINT landing_pages_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.lead_share_transactions.lead_share_transactions_from_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'lead_share_transactions_from_business_id_fkey' AND conrelid = 'public.lead_share_transactions'::regclass
    ) THEN
        ALTER TABLE public.lead_share_transactions ADD CONSTRAINT lead_share_transactions_from_business_id_fkey FOREIGN KEY (from_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.lead_share_transactions.lead_share_transactions_lead_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'lead_share_transactions_lead_id_fkey' AND conrelid = 'public.lead_share_transactions'::regclass
    ) THEN
        ALTER TABLE public.lead_share_transactions ADD CONSTRAINT lead_share_transactions_lead_id_fkey FOREIGN KEY (lead_id) REFERENCES public.shared_leads(id);
    END IF;
END
$md_bl$;

-- public.lead_share_transactions.lead_share_transactions_to_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'lead_share_transactions_to_business_id_fkey' AND conrelid = 'public.lead_share_transactions'::regclass
    ) THEN
        ALTER TABLE public.lead_share_transactions ADD CONSTRAINT lead_share_transactions_to_business_id_fkey FOREIGN KEY (to_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.link_clicks.link_clicks_link_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'link_clicks_link_id_fkey' AND conrelid = 'public.link_clicks'::regclass
    ) THEN
        ALTER TABLE public.link_clicks ADD CONSTRAINT link_clicks_link_id_fkey FOREIGN KEY (link_id) REFERENCES public.tracked_links(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_activity.loyalty_activity_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_activity_member_id_fkey' AND conrelid = 'public.loyalty_activity'::regclass
    ) THEN
        ALTER TABLE public.loyalty_activity ADD CONSTRAINT loyalty_activity_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_checkins.loyalty_checkins_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_checkins_member_id_fkey' AND conrelid = 'public.loyalty_checkins'::regclass
    ) THEN
        ALTER TABLE public.loyalty_checkins ADD CONSTRAINT loyalty_checkins_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_enrollments.loyalty_enrollments_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_enrollments_program_id_fkey' AND conrelid = 'public.loyalty_enrollments'::regclass
    ) THEN
        ALTER TABLE public.loyalty_enrollments ADD CONSTRAINT loyalty_enrollments_program_id_fkey FOREIGN KEY (program_id) REFERENCES public.loyalty_programs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_members.loyalty_members_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_members_network_id_fkey' AND conrelid = 'public.loyalty_members'::regclass
    ) THEN
        ALTER TABLE public.loyalty_members ADD CONSTRAINT loyalty_members_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.loyalty_members.loyalty_members_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_members_program_id_fkey' AND conrelid = 'public.loyalty_members'::regclass
    ) THEN
        ALTER TABLE public.loyalty_members ADD CONSTRAINT loyalty_members_program_id_fkey FOREIGN KEY (program_id) REFERENCES public.loyalty_programs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_members.loyalty_members_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_members_visitor_account_id_fkey' AND conrelid = 'public.loyalty_members'::regclass
    ) THEN
        ALTER TABLE public.loyalty_members ADD CONSTRAINT loyalty_members_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_milestones_completed.loyalty_milestones_completed_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_milestones_completed_member_id_fkey' AND conrelid = 'public.loyalty_milestones_completed'::regclass
    ) THEN
        ALTER TABLE public.loyalty_milestones_completed ADD CONSTRAINT loyalty_milestones_completed_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_milestones_completed.loyalty_milestones_completed_milestone_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_milestones_completed_milestone_id_fkey' AND conrelid = 'public.loyalty_milestones_completed'::regclass
    ) THEN
        ALTER TABLE public.loyalty_milestones_completed ADD CONSTRAINT loyalty_milestones_completed_milestone_id_fkey FOREIGN KEY (milestone_id) REFERENCES public.loyalty_milestones(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_milestones.loyalty_milestones_loyalty_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_milestones_loyalty_program_id_fkey' AND conrelid = 'public.loyalty_milestones'::regclass
    ) THEN
        ALTER TABLE public.loyalty_milestones ADD CONSTRAINT loyalty_milestones_loyalty_program_id_fkey FOREIGN KEY (loyalty_program_id) REFERENCES public.loyalty_programs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_programs.loyalty_programs_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_programs_directory_id_fkey' AND conrelid = 'public.loyalty_programs'::regclass
    ) THEN
        ALTER TABLE public.loyalty_programs ADD CONSTRAINT loyalty_programs_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_programs.loyalty_programs_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_programs_network_id_fkey' AND conrelid = 'public.loyalty_programs'::regclass
    ) THEN
        ALTER TABLE public.loyalty_programs ADD CONSTRAINT loyalty_programs_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.loyalty_reward_tiers.loyalty_reward_tiers_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_reward_tiers_program_id_fkey' AND conrelid = 'public.loyalty_reward_tiers'::regclass
    ) THEN
        ALTER TABLE public.loyalty_reward_tiers ADD CONSTRAINT loyalty_reward_tiers_program_id_fkey FOREIGN KEY (program_id) REFERENCES public.loyalty_programs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_rewards_earned.loyalty_rewards_earned_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_rewards_earned_member_id_fkey' AND conrelid = 'public.loyalty_rewards_earned'::regclass
    ) THEN
        ALTER TABLE public.loyalty_rewards_earned ADD CONSTRAINT loyalty_rewards_earned_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_rewards_earned.loyalty_rewards_earned_tier_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_rewards_earned_tier_id_fkey' AND conrelid = 'public.loyalty_rewards_earned'::regclass
    ) THEN
        ALTER TABLE public.loyalty_rewards_earned ADD CONSTRAINT loyalty_rewards_earned_tier_id_fkey FOREIGN KEY (tier_id) REFERENCES public.loyalty_reward_tiers(id);
    END IF;
END
$md_bl$;

-- public.loyalty_scans.loyalty_scans_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_scans_business_id_fkey' AND conrelid = 'public.loyalty_scans'::regclass
    ) THEN
        ALTER TABLE public.loyalty_scans ADD CONSTRAINT loyalty_scans_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.loyalty_scans.loyalty_scans_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_scans_member_id_fkey' AND conrelid = 'public.loyalty_scans'::regclass
    ) THEN
        ALTER TABLE public.loyalty_scans ADD CONSTRAINT loyalty_scans_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_scans.loyalty_scans_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_scans_program_id_fkey' AND conrelid = 'public.loyalty_scans'::regclass
    ) THEN
        ALTER TABLE public.loyalty_scans ADD CONSTRAINT loyalty_scans_program_id_fkey FOREIGN KEY (program_id) REFERENCES public.loyalty_programs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.loyalty_tiers.loyalty_tiers_loyalty_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_tiers_loyalty_program_id_fkey' AND conrelid = 'public.loyalty_tiers'::regclass
    ) THEN
        ALTER TABLE public.loyalty_tiers ADD CONSTRAINT loyalty_tiers_loyalty_program_id_fkey FOREIGN KEY (loyalty_program_id) REFERENCES public.loyalty_programs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.network_branding.network_branding_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'network_branding_network_id_fkey' AND conrelid = 'public.network_branding'::regclass
    ) THEN
        ALTER TABLE public.network_branding ADD CONSTRAINT network_branding_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.networks.networks_owner_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'networks_owner_id_fkey' AND conrelid = 'public.networks'::regclass
    ) THEN
        ALTER TABLE public.networks ADD CONSTRAINT networks_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES public.users(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.newsletter_digests.newsletter_digests_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_digests_directory_id_fkey' AND conrelid = 'public.newsletter_digests'::regclass
    ) THEN
        ALTER TABLE public.newsletter_digests ADD CONSTRAINT newsletter_digests_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.newsletter_queue.newsletter_queue_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_queue_directory_id_fkey' AND conrelid = 'public.newsletter_queue'::regclass
    ) THEN
        ALTER TABLE public.newsletter_queue ADD CONSTRAINT newsletter_queue_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.newsletter_subscribers.newsletter_subscribers_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'newsletter_subscribers_directory_id_fkey' AND conrelid = 'public.newsletter_subscribers'::regclass
    ) THEN
        ALTER TABLE public.newsletter_subscribers ADD CONSTRAINT newsletter_subscribers_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.offer_claims.offer_claims_offer_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'offer_claims_offer_id_fkey' AND conrelid = 'public.offer_claims'::regclass
    ) THEN
        ALTER TABLE public.offer_claims ADD CONSTRAINT offer_claims_offer_id_fkey FOREIGN KEY (offer_id) REFERENCES public.claim_offers(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.password_resets.password_resets_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'password_resets_user_id_fkey' AND conrelid = 'public.password_resets'::regclass
    ) THEN
        ALTER TABLE public.password_resets ADD CONSTRAINT password_resets_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.pay_per_call.pay_per_call_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'pay_per_call_business_id_fkey' AND conrelid = 'public.pay_per_call'::regclass
    ) THEN
        ALTER TABLE public.pay_per_call ADD CONSTRAINT pay_per_call_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.pay_per_call.pay_per_call_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'pay_per_call_directory_id_fkey' AND conrelid = 'public.pay_per_call'::regclass
    ) THEN
        ALTER TABLE public.pay_per_call ADD CONSTRAINT pay_per_call_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.plan_slot_bookings.plan_slot_bookings_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'plan_slot_bookings_business_id_fkey' AND conrelid = 'public.plan_slot_bookings'::regclass
    ) THEN
        ALTER TABLE public.plan_slot_bookings ADD CONSTRAINT plan_slot_bookings_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.plan_slot_bookings.plan_slot_bookings_plan_tier_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'plan_slot_bookings_plan_tier_id_fkey' AND conrelid = 'public.plan_slot_bookings'::regclass
    ) THEN
        ALTER TABLE public.plan_slot_bookings ADD CONSTRAINT plan_slot_bookings_plan_tier_id_fkey FOREIGN KEY (plan_tier_id) REFERENCES public.plan_tiers(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.point_issuance_log.point_issuance_log_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_issuance_log_member_id_fkey' AND conrelid = 'public.point_issuance_log'::regclass
    ) THEN
        ALTER TABLE public.point_issuance_log ADD CONSTRAINT point_issuance_log_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.point_issuance_log.point_issuance_log_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_issuance_log_network_id_fkey' AND conrelid = 'public.point_issuance_log'::regclass
    ) THEN
        ALTER TABLE public.point_issuance_log ADD CONSTRAINT point_issuance_log_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.point_issuance_log.point_issuance_log_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_issuance_log_program_id_fkey' AND conrelid = 'public.point_issuance_log'::regclass
    ) THEN
        ALTER TABLE public.point_issuance_log ADD CONSTRAINT point_issuance_log_program_id_fkey FOREIGN KEY (program_id) REFERENCES public.loyalty_programs(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.point_redemption_log.point_redemption_log_member_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_redemption_log_member_id_fkey' AND conrelid = 'public.point_redemption_log'::regclass
    ) THEN
        ALTER TABLE public.point_redemption_log ADD CONSTRAINT point_redemption_log_member_id_fkey FOREIGN KEY (member_id) REFERENCES public.loyalty_members(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.point_redemption_log.point_redemption_log_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_redemption_log_network_id_fkey' AND conrelid = 'public.point_redemption_log'::regclass
    ) THEN
        ALTER TABLE public.point_redemption_log ADD CONSTRAINT point_redemption_log_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.point_redemption_log.point_redemption_log_program_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_redemption_log_program_id_fkey' AND conrelid = 'public.point_redemption_log'::regclass
    ) THEN
        ALTER TABLE public.point_redemption_log ADD CONSTRAINT point_redemption_log_program_id_fkey FOREIGN KEY (program_id) REFERENCES public.loyalty_programs(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.point_treasury.point_treasury_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'point_treasury_network_id_fkey' AND conrelid = 'public.point_treasury'::regclass
    ) THEN
        ALTER TABLE public.point_treasury ADD CONSTRAINT point_treasury_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.poll_votes.poll_votes_poll_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'poll_votes_poll_id_fkey' AND conrelid = 'public.poll_votes'::regclass
    ) THEN
        ALTER TABLE public.poll_votes ADD CONSTRAINT poll_votes_poll_id_fkey FOREIGN KEY (poll_id) REFERENCES public.polls(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.poll_votes.poll_votes_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'poll_votes_visitor_account_id_fkey' AND conrelid = 'public.poll_votes'::regclass
    ) THEN
        ALTER TABLE public.poll_votes ADD CONSTRAINT poll_votes_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.polls.polls_created_by_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'polls_created_by_fkey' AND conrelid = 'public.polls'::regclass
    ) THEN
        ALTER TABLE public.polls ADD CONSTRAINT polls_created_by_fkey FOREIGN KEY (created_by) REFERENCES public.users(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.polls.polls_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'polls_directory_id_fkey' AND conrelid = 'public.polls'::regclass
    ) THEN
        ALTER TABLE public.polls ADD CONSTRAINT polls_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.price_bundles.price_bundles_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'price_bundles_directory_id_fkey' AND conrelid = 'public.price_bundles'::regclass
    ) THEN
        ALTER TABLE public.price_bundles ADD CONSTRAINT price_bundles_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.price_bundles.price_bundles_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'price_bundles_network_id_fkey' AND conrelid = 'public.price_bundles'::regclass
    ) THEN
        ALTER TABLE public.price_bundles ADD CONSTRAINT price_bundles_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.programmatic_pages.programmatic_pages_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'programmatic_pages_directory_id_fkey' AND conrelid = 'public.programmatic_pages'::regclass
    ) THEN
        ALTER TABLE public.programmatic_pages ADD CONSTRAINT programmatic_pages_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.programmatic_pages.programmatic_pages_location_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'programmatic_pages_location_id_fkey' AND conrelid = 'public.programmatic_pages'::regclass
    ) THEN
        ALTER TABLE public.programmatic_pages ADD CONSTRAINT programmatic_pages_location_id_fkey FOREIGN KEY (location_id) REFERENCES public.directory_locations(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.programmatic_pages.programmatic_pages_service_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'programmatic_pages_service_id_fkey' AND conrelid = 'public.programmatic_pages'::regclass
    ) THEN
        ALTER TABLE public.programmatic_pages ADD CONSTRAINT programmatic_pages_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.directory_services(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.provider_keys.provider_keys_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_keys_directory_id_fkey' AND conrelid = 'public.provider_keys'::regclass
    ) THEN
        ALTER TABLE public.provider_keys ADD CONSTRAINT provider_keys_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.provider_keys.provider_keys_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_keys_network_id_fkey' AND conrelid = 'public.provider_keys'::regclass
    ) THEN
        ALTER TABLE public.provider_keys ADD CONSTRAINT provider_keys_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.provider_keys.provider_keys_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_keys_tenant_id_fkey' AND conrelid = 'public.provider_keys'::regclass
    ) THEN
        ALTER TABLE public.provider_keys ADD CONSTRAINT provider_keys_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);
    END IF;
END
$md_bl$;

-- public.public_pages.public_pages_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'public_pages_business_id_fkey' AND conrelid = 'public.public_pages'::regclass
    ) THEN
        ALTER TABLE public.public_pages ADD CONSTRAINT public_pages_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.public_pages.public_pages_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'public_pages_directory_id_fkey' AND conrelid = 'public.public_pages'::regclass
    ) THEN
        ALTER TABLE public.public_pages ADD CONSTRAINT public_pages_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.public_themes.public_themes_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'public_themes_directory_id_fkey' AND conrelid = 'public.public_themes'::regclass
    ) THEN
        ALTER TABLE public.public_themes ADD CONSTRAINT public_themes_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.reviews.reviews_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reviews_business_id_fkey' AND conrelid = 'public.reviews'::regclass
    ) THEN
        ALTER TABLE public.reviews ADD CONSTRAINT reviews_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.reviews.reviews_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reviews_directory_id_fkey' AND conrelid = 'public.reviews'::regclass
    ) THEN
        ALTER TABLE public.reviews ADD CONSTRAINT reviews_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.rfq_bids.rfq_bids_bidder_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfq_bids_bidder_business_id_fkey' AND conrelid = 'public.rfq_bids'::regclass
    ) THEN
        ALTER TABLE public.rfq_bids ADD CONSTRAINT rfq_bids_bidder_business_id_fkey FOREIGN KEY (bidder_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.rfq_bids.rfq_bids_rfq_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfq_bids_rfq_id_fkey' AND conrelid = 'public.rfq_bids'::regclass
    ) THEN
        ALTER TABLE public.rfq_bids ADD CONSTRAINT rfq_bids_rfq_id_fkey FOREIGN KEY (rfq_id) REFERENCES public.rfqs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.rfq_messages.rfq_messages_rfq_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfq_messages_rfq_id_fkey' AND conrelid = 'public.rfq_messages'::regclass
    ) THEN
        ALTER TABLE public.rfq_messages ADD CONSTRAINT rfq_messages_rfq_id_fkey FOREIGN KEY (rfq_id) REFERENCES public.rfqs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.rfq_messages.rfq_messages_sender_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfq_messages_sender_business_id_fkey' AND conrelid = 'public.rfq_messages'::regclass
    ) THEN
        ALTER TABLE public.rfq_messages ADD CONSTRAINT rfq_messages_sender_business_id_fkey FOREIGN KEY (sender_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.rfqs.rfqs_awarded_to_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfqs_awarded_to_fkey' AND conrelid = 'public.rfqs'::regclass
    ) THEN
        ALTER TABLE public.rfqs ADD CONSTRAINT rfqs_awarded_to_fkey FOREIGN KEY (awarded_to) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.rfqs.rfqs_poster_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'rfqs_poster_business_id_fkey' AND conrelid = 'public.rfqs'::regclass
    ) THEN
        ALTER TABLE public.rfqs ADD CONSTRAINT rfqs_poster_business_id_fkey FOREIGN KEY (poster_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.schema_config.schema_config_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'schema_config_directory_id_fkey' AND conrelid = 'public.schema_config'::regclass
    ) THEN
        ALTER TABLE public.schema_config ADD CONSTRAINT schema_config_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.search_config.search_config_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'search_config_directory_id_fkey' AND conrelid = 'public.search_config'::regclass
    ) THEN
        ALTER TABLE public.search_config ADD CONSTRAINT search_config_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.seo_fallback_templates.seo_fallback_templates_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seo_fallback_templates_directory_id_fkey' AND conrelid = 'public.seo_fallback_templates'::regclass
    ) THEN
        ALTER TABLE public.seo_fallback_templates ADD CONSTRAINT seo_fallback_templates_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.service_bookings.service_bookings_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_bookings_business_id_fkey' AND conrelid = 'public.service_bookings'::regclass
    ) THEN
        ALTER TABLE public.service_bookings ADD CONSTRAINT service_bookings_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.service_bookings.service_bookings_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_bookings_directory_id_fkey' AND conrelid = 'public.service_bookings'::regclass
    ) THEN
        ALTER TABLE public.service_bookings ADD CONSTRAINT service_bookings_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.service_bookings.service_bookings_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_bookings_visitor_account_id_fkey' AND conrelid = 'public.service_bookings'::regclass
    ) THEN
        ALTER TABLE public.service_bookings ADD CONSTRAINT service_bookings_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.service_prices.service_prices_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_prices_directory_id_fkey' AND conrelid = 'public.service_prices'::regclass
    ) THEN
        ALTER TABLE public.service_prices ADD CONSTRAINT service_prices_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.service_prices.service_prices_network_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'service_prices_network_id_fkey' AND conrelid = 'public.service_prices'::regclass
    ) THEN
        ALTER TABLE public.service_prices ADD CONSTRAINT service_prices_network_id_fkey FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.settlement_invoices.settlement_invoices_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_invoices_business_id_fkey' AND conrelid = 'public.settlement_invoices'::regclass
    ) THEN
        ALTER TABLE public.settlement_invoices ADD CONSTRAINT settlement_invoices_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.settlement_invoices.settlement_invoices_run_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_invoices_run_id_fkey' AND conrelid = 'public.settlement_invoices'::regclass
    ) THEN
        ALTER TABLE public.settlement_invoices ADD CONSTRAINT settlement_invoices_run_id_fkey FOREIGN KEY (run_id) REFERENCES public.settlement_runs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.settlement_payouts.settlement_payouts_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_payouts_business_id_fkey' AND conrelid = 'public.settlement_payouts'::regclass
    ) THEN
        ALTER TABLE public.settlement_payouts ADD CONSTRAINT settlement_payouts_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.settlement_payouts.settlement_payouts_run_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'settlement_payouts_run_id_fkey' AND conrelid = 'public.settlement_payouts'::regclass
    ) THEN
        ALTER TABLE public.settlement_payouts ADD CONSTRAINT settlement_payouts_run_id_fkey FOREIGN KEY (run_id) REFERENCES public.settlement_runs(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.shared_leads.shared_leads_claimed_by_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'shared_leads_claimed_by_fkey' AND conrelid = 'public.shared_leads'::regclass
    ) THEN
        ALTER TABLE public.shared_leads ADD CONSTRAINT shared_leads_claimed_by_fkey FOREIGN KEY (claimed_by) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.shared_leads.shared_leads_poster_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'shared_leads_poster_business_id_fkey' AND conrelid = 'public.shared_leads'::regclass
    ) THEN
        ALTER TABLE public.shared_leads ADD CONSTRAINT shared_leads_poster_business_id_fkey FOREIGN KEY (poster_business_id) REFERENCES public.businesses(id);
    END IF;
END
$md_bl$;

-- public.sitemap_config.sitemap_config_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sitemap_config_directory_id_fkey' AND conrelid = 'public.sitemap_config'::regclass
    ) THEN
        ALTER TABLE public.sitemap_config ADD CONSTRAINT sitemap_config_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.sponsored_listings.sponsored_listings_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsored_listings_business_id_fkey' AND conrelid = 'public.sponsored_listings'::regclass
    ) THEN
        ALTER TABLE public.sponsored_listings ADD CONSTRAINT sponsored_listings_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.sponsored_listings.sponsored_listings_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsored_listings_directory_id_fkey' AND conrelid = 'public.sponsored_listings'::regclass
    ) THEN
        ALTER TABLE public.sponsored_listings ADD CONSTRAINT sponsored_listings_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.sponsors.sponsors_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsors_business_id_fkey' AND conrelid = 'public.sponsors'::regclass
    ) THEN
        ALTER TABLE public.sponsors ADD CONSTRAINT sponsors_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.sponsors.sponsors_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sponsors_directory_id_fkey' AND conrelid = 'public.sponsors'::regclass
    ) THEN
        ALTER TABLE public.sponsors ADD CONSTRAINT sponsors_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.submissions.submissions_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'submissions_directory_id_fkey' AND conrelid = 'public.submissions'::regclass
    ) THEN
        ALTER TABLE public.submissions ADD CONSTRAINT submissions_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.supplier_products.supplier_products_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'supplier_products_business_id_fkey' AND conrelid = 'public.supplier_products'::regclass
    ) THEN
        ALTER TABLE public.supplier_products ADD CONSTRAINT supplier_products_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.survey_responses.survey_responses_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'survey_responses_directory_id_fkey' AND conrelid = 'public.survey_responses'::regclass
    ) THEN
        ALTER TABLE public.survey_responses ADD CONSTRAINT survey_responses_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.survey_responses.survey_responses_survey_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'survey_responses_survey_id_fkey' AND conrelid = 'public.survey_responses'::regclass
    ) THEN
        ALTER TABLE public.survey_responses ADD CONSTRAINT survey_responses_survey_id_fkey FOREIGN KEY (survey_id) REFERENCES public.directory_surveys(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.survey_responses.survey_responses_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'survey_responses_visitor_account_id_fkey' AND conrelid = 'public.survey_responses'::regclass
    ) THEN
        ALTER TABLE public.survey_responses ADD CONSTRAINT survey_responses_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.tag_rules.tag_rules_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tag_rules_tenant_id_fkey' AND conrelid = 'public.tag_rules'::regclass
    ) THEN
        ALTER TABLE public.tag_rules ADD CONSTRAINT tag_rules_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.topic_format_templates.topic_format_templates_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'topic_format_templates_directory_id_fkey' AND conrelid = 'public.topic_format_templates'::regclass
    ) THEN
        ALTER TABLE public.topic_format_templates ADD CONSTRAINT topic_format_templates_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.tracked_links.tracked_links_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'tracked_links_tenant_id_fkey' AND conrelid = 'public.tracked_links'::regclass
    ) THEN
        ALTER TABLE public.tracked_links ADD CONSTRAINT tracked_links_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.trap_door_templates.trap_door_templates_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'trap_door_templates_directory_id_fkey' AND conrelid = 'public.trap_door_templates'::regclass
    ) THEN
        ALTER TABLE public.trap_door_templates ADD CONSTRAINT trap_door_templates_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.twilio_numbers.twilio_numbers_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'twilio_numbers_directory_id_fkey' AND conrelid = 'public.twilio_numbers'::regclass
    ) THEN
        ALTER TABLE public.twilio_numbers ADD CONSTRAINT twilio_numbers_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.users.users_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'users_tenant_id_fkey' AND conrelid = 'public.users'::regclass
    ) THEN
        ALTER TABLE public.users ADD CONSTRAINT users_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.visitor_accounts.visitor_accounts_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_accounts_directory_id_fkey' AND conrelid = 'public.visitor_accounts'::regclass
    ) THEN
        ALTER TABLE public.visitor_accounts ADD CONSTRAINT visitor_accounts_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.visitor_events.visitor_events_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_events_business_id_fkey' AND conrelid = 'public.visitor_events'::regclass
    ) THEN
        ALTER TABLE public.visitor_events ADD CONSTRAINT visitor_events_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.visitor_events.visitor_events_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_events_directory_id_fkey' AND conrelid = 'public.visitor_events'::regclass
    ) THEN
        ALTER TABLE public.visitor_events ADD CONSTRAINT visitor_events_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.visitor_events.visitor_events_session_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_events_session_id_fkey' AND conrelid = 'public.visitor_events'::regclass
    ) THEN
        ALTER TABLE public.visitor_events ADD CONSTRAINT visitor_events_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.visitor_sessions(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.visitor_events.visitor_events_visitor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_events_visitor_id_fkey' AND conrelid = 'public.visitor_events'::regclass
    ) THEN
        ALTER TABLE public.visitor_events ADD CONSTRAINT visitor_events_visitor_id_fkey FOREIGN KEY (visitor_id) REFERENCES public.visitors(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.visitor_favorites.visitor_favorites_business_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_favorites_business_id_fkey' AND conrelid = 'public.visitor_favorites'::regclass
    ) THEN
        ALTER TABLE public.visitor_favorites ADD CONSTRAINT visitor_favorites_business_id_fkey FOREIGN KEY (business_id) REFERENCES public.businesses(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.visitor_favorites.visitor_favorites_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_favorites_directory_id_fkey' AND conrelid = 'public.visitor_favorites'::regclass
    ) THEN
        ALTER TABLE public.visitor_favorites ADD CONSTRAINT visitor_favorites_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.visitor_favorites.visitor_favorites_visitor_account_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_favorites_visitor_account_id_fkey' AND conrelid = 'public.visitor_favorites'::regclass
    ) THEN
        ALTER TABLE public.visitor_favorites ADD CONSTRAINT visitor_favorites_visitor_account_id_fkey FOREIGN KEY (visitor_account_id) REFERENCES public.visitor_accounts(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.visitor_sessions.visitor_sessions_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_sessions_directory_id_fkey' AND conrelid = 'public.visitor_sessions'::regclass
    ) THEN
        ALTER TABLE public.visitor_sessions ADD CONSTRAINT visitor_sessions_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE SET NULL;
    END IF;
END
$md_bl$;

-- public.visitor_sessions.visitor_sessions_visitor_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'visitor_sessions_visitor_id_fkey' AND conrelid = 'public.visitor_sessions'::regclass
    ) THEN
        ALTER TABLE public.visitor_sessions ADD CONSTRAINT visitor_sessions_visitor_id_fkey FOREIGN KEY (visitor_id) REFERENCES public.visitors(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.webhook_deliveries.webhook_deliveries_webhook_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'webhook_deliveries_webhook_id_fkey' AND conrelid = 'public.webhook_deliveries'::regclass
    ) THEN
        ALTER TABLE public.webhook_deliveries ADD CONSTRAINT webhook_deliveries_webhook_id_fkey FOREIGN KEY (webhook_id) REFERENCES public.webhooks(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

-- public.webhooks.webhooks_directory_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'webhooks_directory_id_fkey' AND conrelid = 'public.webhooks'::regclass
    ) THEN
        ALTER TABLE public.webhooks ADD CONSTRAINT webhooks_directory_id_fkey FOREIGN KEY (directory_id) REFERENCES public.directories(id);
    END IF;
END
$md_bl$;

-- public.webhooks.webhooks_tenant_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'webhooks_tenant_id_fkey' AND conrelid = 'public.webhooks'::regclass
    ) THEN
        ALTER TABLE public.webhooks ADD CONSTRAINT webhooks_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);
    END IF;
END
$md_bl$;

-- public.webhooks.webhooks_user_id_fkey
DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'webhooks_user_id_fkey' AND conrelid = 'public.webhooks'::regclass
    ) THEN
        ALTER TABLE public.webhooks ADD CONSTRAINT webhooks_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);
    END IF;
END
$md_bl$;
