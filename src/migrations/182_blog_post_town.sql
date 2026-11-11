-- W30(b): an article belongs to ONE town of its directory.
--
-- town_key is the same key `satellite_towns.town_key` uses (the lower-cased first
-- comma-part of a listing's city, e.g. "melbourne"), so the town page, the admin
-- town tabs and this column can never disagree about which town a record is about.
-- NULL means the article is city-wide (the historic behaviour every existing post keeps).
--
-- No foreign key: the directory's OWN town is a legitimate target but is not a
-- satellite_towns row. The write path validates the key against the directory's
-- local towns instead, so a typo can never create an orphan label.
ALTER TABLE blog_posts ADD COLUMN IF NOT EXISTS town_key text;

CREATE INDEX IF NOT EXISTS idx_blog_posts_directory_town
    ON blog_posts (directory_id, town_key)
    WHERE town_key IS NOT NULL AND town_key <> '';
