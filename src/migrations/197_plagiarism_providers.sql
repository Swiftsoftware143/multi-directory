-- Plagiarism-check providers for the blog guardrail (David, Nov 15 2026).
--
-- The admin Guardrails tab already offered a provider picker (Winston AI, Copyleaks,
-- Originality.ai, PlagiarismCheck.org, Copyscape) with thresholds 5% hold / 15% block, but the
-- picker's names were never registered in `available_providers`, so the Integration Center (which
-- renders its list from that table) had no entry under which to store a key — a key could never
-- actually be entered. Registering the five names here closes that gap. Only Winston AI has a
-- wired adapter today (the one David chose); the rest are listed so the picker and the credential
-- screen line up, and report "adapter not wired yet" until their adapters land.

INSERT INTO available_providers
    (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only)
VALUES
    ('winston',         'Winston AI',         'Plagiarism check for blog drafts (Winston AI /v2/plagiarism)',  false, '[]', '📄', NULL, NULL, false),
    ('copyleaks',       'Copyleaks',          'Plagiarism detection for blog drafts',                          false, '[]', '📄', NULL, NULL, false),
    ('originality',     'Originality.ai',     'AI-content and plagiarism detection for blog drafts',           false, '[]', '📄', NULL, NULL, false),
    ('plagiarismcheck', 'PlagiarismCheck.org','Plagiarism detection for blog drafts',                           false, '[]', '📄', NULL, NULL, false),
    ('copyscape',       'Copyscape',          'Plagiarism detection for blog drafts (Copyscape Premium API)',   false, '[]', '📄', NULL, NULL, false)
ON CONFLICT (key) DO NOTHING;
