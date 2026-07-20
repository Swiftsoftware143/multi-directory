-- Onboarding survey configuration (one per directory)
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

CREATE INDEX IF NOT EXISTS idx_dir_surveys_directory ON directory_surveys(directory_id);

-- Survey responses per visitor
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

CREATE INDEX IF NOT EXISTS idx_survey_responses_survey ON survey_responses(survey_id);
CREATE INDEX IF NOT EXISTS idx_survey_responses_visitor ON survey_responses(visitor_account_id);
CREATE INDEX IF NOT EXISTS idx_survey_responses_directory ON survey_responses(directory_id);

-- Add interest_tags column to visitor_accounts if not exists
ALTER TABLE visitor_accounts ADD COLUMN IF NOT EXISTS interest_tags TEXT[] NOT NULL DEFAULT '{}';
