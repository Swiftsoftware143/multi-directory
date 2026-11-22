-- 202_seo_desc_state_dedup.sql
--
-- Work-queue line 72 gap 2 (SEO): the category and city meta descriptions render a
-- redundant state double-mention — "Find Accountant businesses and services in
-- Palm Bay, FL, Florida." — because the {{city}} merge field already resolves to
-- "<City>, <ST>" (directories.city stores the locality WITH its state abbreviation)
-- while the seeded templates ALSO hard-coded ", Florida". One mention is correct:
-- keep the abbreviation {{city}} carries and strip the hard-coded state name.
--
-- Idempotent: after the replace, no description_template still contains
-- "{{city}}, Florida", so the WHERE guard never re-selects a row.

UPDATE seo_fallback_templates
   SET description_template = replace(description_template, '{{city}}, Florida', '{{city}}'),
       updated_at = NOW()
 WHERE page_type IN ('category', 'city')
   AND description_template LIKE '%{{city}}, Florida%';
