-- Rankifyr's agreed defaults (David, Nov 14 2026):
--   * one auto-published post per business per day (the admin may raise it to 3 once the
--     business's posts start appearing in Google);
--   * quality score stays at 70 and is revisited after the first 50 automatic posts;
--   * plagiarism: over 5% matching text is held, over 15% is blocked.

ALTER TABLE directories
    ALTER COLUMN blog_auto_publish_daily_cap SET DEFAULT 1;

UPDATE directories
SET blog_auto_publish_daily_cap = 1
WHERE blog_auto_publish_daily_cap = 3;

UPDATE directories
SET blog_guardrail_settings = blog_guardrail_settings
    || '{"plagiarism_hold_pct": 5, "plagiarism_block_pct": 15, "plagiarism_provider": ""}'::jsonb
WHERE NOT (blog_guardrail_settings ? 'plagiarism_hold_pct');
