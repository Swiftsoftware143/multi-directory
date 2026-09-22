-- 101: In-house onboarding — the admin questionnaire builder (card B65).
--
-- Decision (David, 2026-09-22): onboarding belongs to Multi-Directory. Routing a signup
-- questionnaire through IncentiveSwift "really doesn't make any sense". MD stores the
-- questionnaire and the answers, credits its own native currency, and maps the answers
-- into CoreSwift through the existing coreswift seam. The IncentiveSwift fire-and-forget
-- call is deleted from the code path.
--
-- Questionnaire model (this is what makes the builder possible):
--   * one questionnaire PER AUDIENCE per directory — customer / supplier / business, and
--     any audience added later (the CHECK is the only place the list lives);
--   * draft vs published, so a half-written questionnaire never reaches the public;
--   * reward_units — the native currency credited on completion, set by the admin;
--   * questions stay JSONB (authored from the UI, no SQL and no code change to add one),
--     but their shape is now fixed by the builder contract:
--       {id, type, label, help_text, required, options[], scale_min, scale_max, order}
--     with type in short_text, long_text, single_choice, multiple_choice, dropdown,
--     number, yes_no, rating, date.
--
-- Idempotent: ADD COLUMN IF NOT EXISTS / DROP CONSTRAINT IF EXISTS everywhere, and the
-- backfills only touch rows they need to.

-- ── directory_surveys: audience, lifecycle, reward ────────────────────────────
ALTER TABLE directory_surveys ADD COLUMN IF NOT EXISTS audience text NOT NULL DEFAULT 'customer';
ALTER TABLE directory_surveys ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'draft';
ALTER TABLE directory_surveys ADD COLUMN IF NOT EXISTS reward_units integer NOT NULL DEFAULT 0;
ALTER TABLE directory_surveys ADD COLUMN IF NOT EXISTS network_id uuid REFERENCES networks(id) ON DELETE SET NULL;
ALTER TABLE directory_surveys ADD COLUMN IF NOT EXISTS published_at timestamptz;

-- The rows that already existed were the live customer onboarding questionnaires, so they
-- are published, not drafts (they answered 200 to the public GET before this migration).
UPDATE directory_surveys SET status = 'published' WHERE enabled = true AND status <> 'published';

ALTER TABLE directory_surveys DROP CONSTRAINT IF EXISTS directory_surveys_audience_check;
ALTER TABLE directory_surveys ADD CONSTRAINT directory_surveys_audience_check
    CHECK (audience IN ('customer', 'supplier', 'business'));

ALTER TABLE directory_surveys DROP CONSTRAINT IF EXISTS directory_surveys_status_check;
ALTER TABLE directory_surveys ADD CONSTRAINT directory_surveys_status_check
    CHECK (status IN ('draft', 'published'));

-- One questionnaire per directory per audience: the builder edits a single draft/published
-- pair rather than accumulating duplicates (and the public GET is then unambiguous).
CREATE UNIQUE INDEX IF NOT EXISTS uq_directory_surveys_directory_audience
    ON directory_surveys (directory_id, audience);

CREATE INDEX IF NOT EXISTS idx_directory_surveys_network ON directory_surveys (network_id);

COMMENT ON COLUMN directory_surveys.audience IS
    'Who the questionnaire is for: customer | supplier | business. Authored per directory in the admin panel.';
COMMENT ON COLUMN directory_surveys.status IS
    'draft | published. Only a published questionnaire is served to the public; draft never reaches a visitor.';
COMMENT ON COLUMN directory_surveys.questions IS
    'JSONB array of {id,type,label,help_text,required,options[],scale_min,scale_max,order}. Types: short_text,long_text,single_choice,multiple_choice,dropdown,number,yes_no,rating,date.';
COMMENT ON COLUMN directory_surveys.reward_units IS
    'Native currency units credited on completion (100 units = US$1). 0 = answers are stored but nothing is earned.';

-- ── survey_responses: audience + what really happened ─────────────────────────
ALTER TABLE survey_responses ADD COLUMN IF NOT EXISTS audience text NOT NULL DEFAULT 'customer';
ALTER TABLE survey_responses ADD COLUMN IF NOT EXISTS reward_units_awarded integer NOT NULL DEFAULT 0;
ALTER TABLE survey_responses ADD COLUMN IF NOT EXISTS currency_name text;
ALTER TABLE survey_responses ADD COLUMN IF NOT EXISTS coreswift_pushed boolean NOT NULL DEFAULT false;
ALTER TABLE survey_responses ADD COLUMN IF NOT EXISTS coreswift_push_error text;

CREATE INDEX IF NOT EXISTS idx_survey_responses_completed
    ON survey_responses (directory_id, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_survey_responses_audience
    ON survey_responses (directory_id, audience);

COMMENT ON COLUMN survey_responses.coreswift_pushed IS
    'True only when CoreSwift really accepted the answers. A skipped (CRM not connected) or failed push stays false, and the reason is in coreswift_push_error — never faked.';
COMMENT ON COLUMN survey_responses.reward_units_awarded IS
    'Native currency units actually credited for this response (0 when the respondent could not be identified or the programme awards nothing).';
