-- Migration 076: Native Loyalty Engine (cloned from IncentiveSwift, re-keyed to directory)
-- Loyalty capability is now inherent to every directory. Each directory admin
-- creates/configures their own loyalty programs. Members = visitor_accounts.

-- ============================================================
-- Core program + member + check-in + rewards (re-keyed to directory)
-- ============================================================

CREATE TABLE IF NOT EXISTS loyalty_programs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id uuid NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    name text NOT NULL,
    recognition_method text DEFAULT 'both',
    points_per_checkin integer DEFAULT 10,
    max_checkins_per_day integer DEFAULT 1,
    point_decay_days integer,
    points_expire_days integer DEFAULT 365,
    currency_name text DEFAULT 'Points',
    currency_icon text DEFAULT '⭐',
    currency_color text DEFAULT '#0d9488',
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
    created_at timestamptz DEFAULT now(),
    updated_at timestamptz DEFAULT now()
);

CREATE TABLE IF NOT EXISTS loyalty_members (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    program_id uuid NOT NULL REFERENCES loyalty_programs(id) ON DELETE CASCADE,
    visitor_account_id uuid NOT NULL REFERENCES visitor_accounts(id) ON DELETE CASCADE,
    points_balance integer DEFAULT 0,
    lifetime_points integer DEFAULT 0,
    tier_id uuid,
    current_streak integer DEFAULT 0,
    longest_streak integer DEFAULT 0,
    last_activity_date timestamptz,
    birthday date,
    referral_code varchar(50) UNIQUE,
    total_referrals integer DEFAULT 0,
    qr_code text,
    qr_code_generated_at timestamptz,
    member_since timestamptz DEFAULT now(),
    last_checkin_at timestamptz,
    UNIQUE (program_id, visitor_account_id)
);

CREATE TABLE IF NOT EXISTS loyalty_checkins (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    member_id uuid NOT NULL REFERENCES loyalty_members(id) ON DELETE CASCADE,
    points_awarded integer NOT NULL,
    method text,
    checked_in_at timestamptz DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS loyalty_checkins_daily_cap
    ON loyalty_checkins (member_id, (checked_in_at::date));

-- ============================================================
-- Tiers (point-level membership tiers)
-- ============================================================
CREATE TABLE IF NOT EXISTS loyalty_tiers (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    loyalty_program_id uuid NOT NULL REFERENCES loyalty_programs(id) ON DELETE CASCADE,
    name varchar(100) NOT NULL,
    min_points bigint NOT NULL DEFAULT 0,
    color varchar(7) NOT NULL DEFAULT '#6B7280',
    perks jsonb DEFAULT '[]'::jsonb,
    multiplier decimal(5,2) NOT NULL DEFAULT 1.0,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- ============================================================
-- Reward catalog + earned rewards (approval lifecycle)
-- ============================================================
CREATE TABLE IF NOT EXISTS loyalty_reward_tiers (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    program_id uuid NOT NULL REFERENCES loyalty_programs(id) ON DELETE CASCADE,
    name text NOT NULL,
    points_required integer NOT NULL,
    requires_approval boolean DEFAULT false,
    reward_tag text NOT NULL,
    marketing_boost jsonb,
    sort_order integer DEFAULT 0
);

CREATE TABLE IF NOT EXISTS loyalty_rewards_earned (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    member_id uuid NOT NULL REFERENCES loyalty_members(id) ON DELETE CASCADE,
    tier_id uuid REFERENCES loyalty_reward_tiers(id),
    status text DEFAULT 'pending',
    earned_at timestamptz DEFAULT now(),
    approved_by uuid,
    fulfilled_at timestamptz
);

-- ============================================================
-- Milestones (trigger-based bonus points/rewards)
-- ============================================================
CREATE TABLE IF NOT EXISTS loyalty_milestones (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    loyalty_program_id uuid NOT NULL REFERENCES loyalty_programs(id) ON DELETE CASCADE,
    name varchar(200) NOT NULL,
    trigger_type varchar(50) NOT NULL,
    trigger_value bigint NOT NULL DEFAULT 0,
    bonus_points bigint NOT NULL DEFAULT 0,
    bonus_reward_id uuid,
    once_per_member boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS loyalty_milestones_completed (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    member_id uuid NOT NULL REFERENCES loyalty_members(id) ON DELETE CASCADE,
    milestone_id uuid NOT NULL REFERENCES loyalty_milestones(id) ON DELETE CASCADE,
    points_awarded bigint NOT NULL DEFAULT 0,
    completed_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (member_id, milestone_id)
);

-- ============================================================
-- Activity ledger (powers dashboards)
-- ============================================================
CREATE TABLE IF NOT EXISTS loyalty_activity (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    member_id uuid NOT NULL REFERENCES loyalty_members(id) ON DELETE CASCADE,
    activity_type varchar(50) NOT NULL,
    description text,
    points_earned bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- ============================================================
-- Enrollment (per-entity opt-in: business / supplier / member)
-- ============================================================
CREATE TABLE IF NOT EXISTS loyalty_enrollments (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    program_id uuid NOT NULL REFERENCES loyalty_programs(id) ON DELETE CASCADE,
    entity_type text NOT NULL,
    entity_id uuid NOT NULL,
    enrolled_at timestamptz DEFAULT now(),
    UNIQUE (program_id, entity_type, entity_id)
);

-- ============================================================
-- Deals + events: LINK to MD's existing tables (no duplicate tables)
-- ============================================================
ALTER TABLE deals ADD COLUMN IF NOT EXISTS loyalty_program_id uuid REFERENCES loyalty_programs(id) ON DELETE SET NULL;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS points_required integer DEFAULT 0;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS redemption_limit integer;
ALTER TABLE deals ADD COLUMN IF NOT EXISTS redemption_count integer DEFAULT 0;

ALTER TABLE community_events ADD COLUMN IF NOT EXISTS loyalty_program_id uuid REFERENCES loyalty_programs(id) ON DELETE SET NULL;
ALTER TABLE community_events ADD COLUMN IF NOT EXISTS event_type text DEFAULT 'general';

-- ============================================================
-- QR scan log
-- ============================================================
CREATE TABLE IF NOT EXISTS loyalty_scans (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    member_id uuid NOT NULL REFERENCES loyalty_members(id) ON DELETE CASCADE,
    program_id uuid NOT NULL REFERENCES loyalty_programs(id) ON DELETE CASCADE,
    business_id uuid REFERENCES businesses(id) ON DELETE SET NULL,
    scan_type text NOT NULL DEFAULT 'checkin',
    points_awarded integer DEFAULT 0,
    metadata jsonb DEFAULT '{}'::jsonb,
    scanned_at timestamptz DEFAULT now()
);

-- ============================================================
-- Indexes
-- ============================================================
CREATE INDEX IF NOT EXISTS idx_loyalty_programs_directory ON loyalty_programs(directory_id);
CREATE INDEX IF NOT EXISTS idx_loyalty_members_program ON loyalty_members(program_id);
CREATE INDEX IF NOT EXISTS idx_loyalty_members_visitor ON loyalty_members(visitor_account_id);
CREATE INDEX IF NOT EXISTS idx_loyalty_tiers_program ON loyalty_tiers(loyalty_program_id);
CREATE INDEX IF NOT EXISTS idx_loyalty_activity_member ON loyalty_activity(member_id);
CREATE INDEX IF NOT EXISTS idx_loyalty_activity_created ON loyalty_activity(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_loyalty_rewards_member ON loyalty_rewards_earned(member_id);
CREATE INDEX IF NOT EXISTS idx_deals_loyalty_program ON deals(loyalty_program_id);
CREATE INDEX IF NOT EXISTS idx_loyalty_scans_program ON loyalty_scans(program_id);
CREATE INDEX IF NOT EXISTS idx_community_events_loyalty ON community_events(loyalty_program_id);
