-- 216_directory_newsletter_registration.sql
--
-- Each city runs its OWN independent newsletter (its own domain, Letterman SaaS). Store the
-- registration URL per directory so the admin panel can set/change it without a code change, and
-- the advert image can link to it. Seeded with the 10 known registration pages; a new city starts
-- NULL and the admin fills it in.

ALTER TABLE directories
    ADD COLUMN IF NOT EXISTS newsletter_registration_url text;

UPDATE directories SET newsletter_registration_url = CASE slug
    WHEN 'palm-bay'      THEN 'https://palmbaypulse.com/register'
    WHEN 'palm-coast'    THEN 'https://palmcoastinsider.com/register'
    WHEN 'st-petersburg' THEN 'https://baybreezebulletin.com/register'
    WHEN 'hollywood'     THEN 'https://broadwalkbeat.com/register'
    WHEN 'pompano-beach' THEN 'https://pompanosheartbeat.com/register'
    WHEN 'boca-raton'    THEN 'https://bocaluxeledger.com/register'
    WHEN 'st-cloud'      THEN 'https://stcloudpulse.com/register'
    WHEN 'lake-nona'     THEN 'https://lakenonaunpacked.com/register'
    WHEN 'winter-garden' THEN 'https://rootedinwintergarden.com/register'
    WHEN 'apopka'        THEN 'https://rootedinapopka.com/register'
    ELSE newsletter_registration_url
END
WHERE slug IN ('palm-bay','palm-coast','st-petersburg','hollywood','pompano-beach',
               'boca-raton','st-cloud','lake-nona','winter-garden','apopka');

COMMENT ON COLUMN directories.newsletter_registration_url IS
    'External /register page for this city''s own newsletter (Letterman). NULL = no newsletter yet.';
