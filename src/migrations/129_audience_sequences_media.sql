-- 129_audience_sequences_media.sql — per-city audience lists, campaign sequences, image media.
--
-- Card: kanban t_63ffc2de (multi-directory: per-city lists & campaign automation).
--
-- The three built-in audience lists already exist on the CoreSwift side and their ids are stored on
-- `directories`/`networks` (coreswift_list_id_claimed / _newsletter / _sponsors). This migration adds
-- the MD-SIDE pieces that were missing:
--   1. campaign_sequences / campaign_sequence_steps — the DEFINITION of a nurture sequence (steps,
--      delay, subject/body, image). MD owns the definition + the audience; CoreSwift owns the SEND
--      (David's binding MAIL decision: MD sends SYSTEM mail only and must not grow a second sender).
--   2. email_campaigns gains the list/segment it targets, so "which sequence is this contact in,
--      per city" is answerable (card item E).
--   3. media_assets — a bookkeeping row per uploaded image so the panel can list/delete them
--      (card item D). The bytes live under the app's uploads root and are served by the app.
--   4. sponsored_listings.crm_pushed_at — records when the Sponsors-list membership push to
--      CoreSwift last succeeded, so the sponsor loop (card item A) is auditable per listing.
--
-- Scope: nothing here is platform-level. A sequence row belongs to a directory OR a network, and a
-- directory's lists/tags are never shared above its network.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

CREATE TABLE IF NOT EXISTS campaign_sequences (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name         text NOT NULL,
    description  text,
    directory_id uuid NULL REFERENCES directories(id) ON DELETE CASCADE,
    network_id   uuid NULL REFERENCES networks(id)   ON DELETE CASCADE,
    target_list  text NOT NULL DEFAULT 'subscribers',
    is_active    boolean NOT NULL DEFAULT true,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE campaign_sequences DROP CONSTRAINT IF EXISTS campaign_sequences_target_list_chk;
ALTER TABLE campaign_sequences ADD CONSTRAINT campaign_sequences_target_list_chk
    CHECK (target_list IN ('subscribers', 'claimed', 'sponsors'));

CREATE INDEX IF NOT EXISTS idx_campaign_sequences_directory ON campaign_sequences (directory_id);
CREATE INDEX IF NOT EXISTS idx_campaign_sequences_network   ON campaign_sequences (network_id);

CREATE TABLE IF NOT EXISTS campaign_sequence_steps (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence_id uuid NOT NULL REFERENCES campaign_sequences(id) ON DELETE CASCADE,
    step_order  integer NOT NULL DEFAULT 1,
    delay_days  integer NOT NULL DEFAULT 0,
    subject     text NOT NULL DEFAULT '',
    body_html   text,
    body_text   text,
    template_id uuid NULL REFERENCES email_templates(id) ON DELETE SET NULL,
    image_url   text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_campaign_sequence_steps_seq
    ON campaign_sequence_steps (sequence_id, step_order);

-- Item E: pin a campaign to the list/segment (and city) it targets.
ALTER TABLE email_campaigns ADD COLUMN IF NOT EXISTS sequence_id         uuid NULL REFERENCES campaign_sequences(id) ON DELETE SET NULL;
ALTER TABLE email_campaigns ADD COLUMN IF NOT EXISTS target_list         text NULL;
ALTER TABLE email_campaigns ADD COLUMN IF NOT EXISTS target_directory_id uuid NULL REFERENCES directories(id) ON DELETE SET NULL;
ALTER TABLE email_campaigns ADD COLUMN IF NOT EXISTS network_id          uuid NULL REFERENCES networks(id)    ON DELETE SET NULL;

-- Item D: one bookkeeping row per uploaded image.
CREATE TABLE IF NOT EXISTS media_assets (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id uuid NULL REFERENCES directories(id) ON DELETE SET NULL,
    filename     text NOT NULL,
    url          text NOT NULL,
    mime_type    text,
    byte_size    bigint NOT NULL DEFAULT 0,
    uploaded_by  uuid NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_media_assets_directory ON media_assets (directory_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_media_assets_url ON media_assets (url);

-- Item A: auditable record of the last successful Sponsors-list push for a sponsored listing.
ALTER TABLE sponsored_listings ADD COLUMN IF NOT EXISTS crm_pushed_at timestamptz NULL;

COMMENT ON TABLE campaign_sequences IS
  'MD-side DEFINITION of a nurture sequence per city/network (kanban t_63ffc2de). CoreSwift owns the send.';
COMMENT ON TABLE campaign_sequence_steps IS
  'Steps of a campaign_sequences row: step_order, delay_days after the previous step, subject/body and an optional image_url from media_assets.';
COMMENT ON COLUMN sponsored_listings.crm_pushed_at IS
  'When the business was last pushed to the directory''s CoreSwift Sponsors list (list 3 of 3), by create/activate.';
