-- B68 — onboarding questionnaire → CoreSwift contact DATA POINTS.
--
-- Per-question target field lives INSIDE directory_surveys.questions[i].coreswift_field
-- (jsonb — no DDL needed; answers stay in survey_responses so re-mapping never loses them).
--
-- The questionnaire-level target list/segment is a real column so the admin's choice
-- persists independently of the question array. It holds a CoreSwift list id (the hub's
-- own id, not a local FK) chosen from the tenant's lists; NULL means "use the audience's
-- default list on the hub".
ALTER TABLE public.directory_surveys
    ADD COLUMN IF NOT EXISTS coreswift_list_id uuid;

COMMENT ON COLUMN public.directory_surveys.coreswift_list_id IS
    'Card B68: CoreSwift list/segment every response of this questionnaire is added to. NULL = the hub''s default audience list. The id is the hub''s own (no local FK).';
