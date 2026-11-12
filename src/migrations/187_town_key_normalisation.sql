-- 187 — one town, ONE key ("St. Cloud" and "St Cloud" were two towns)
--
-- The town key was `lower(trim(city before the comma))`, so one town the importer spelled two
-- ways produced TWO entries in the local area: two chips on the city page ("St. Cloud (283)",
-- "St Cloud (15)") with the listings split between them, and a page of its own for the smaller
-- spelling competing with the real one in search.
--
-- The key is now punctuation-blind — `src/handlers/zaarhub.rs::town_key` and the SQL expression
-- `TRIM(REGEXP_REPLACE(LOWER(SPLIT_PART(TRIM(b.city), ',', 1)), '[^[:alnum:]]+', ' ', 'g'))` —
-- so this file brings the STORED keys to that same form. Every statement is idempotent: a key
-- that is already normalised is untouched, and re-running changes nothing.

-- 1. Approval is per town, not per spelling: if ANY spelling of the town was approved, the
--    surviving row keeps it. (Without this, the merge could silently unpublish a town page.)
UPDATE satellite_towns k
   SET approved = true
 WHERE k.approved = false
   AND EXISTS (
       SELECT 1
         FROM satellite_towns d
        WHERE d.directory_id = k.directory_id
          AND d.id <> k.id
          AND d.approved
          AND TRIM(REGEXP_REPLACE(LOWER(d.town_key), '[^[:alnum:]]+', ' ', 'g'))
              = TRIM(REGEXP_REPLACE(LOWER(k.town_key), '[^[:alnum:]]+', ' ', 'g'))
   );

-- 2. One row per town: keep the oldest (lowest id) of each normalised key per directory.
DELETE FROM satellite_towns d
 WHERE d.id NOT IN (
       SELECT MIN(id)
         FROM satellite_towns
        GROUP BY directory_id, TRIM(REGEXP_REPLACE(LOWER(town_key), '[^[:alnum:]]+', ' ', 'g'))
   );

-- 3. Rewrite the survivors to the normalised key.
UPDATE satellite_towns
   SET town_key = TRIM(REGEXP_REPLACE(LOWER(town_key), '[^[:alnum:]]+', ' ', 'g'))
 WHERE town_key <> TRIM(REGEXP_REPLACE(LOWER(town_key), '[^[:alnum:]]+', ' ', 'g'));

-- 4. Articles filed to a town carry the same key (the town page joins on it).
UPDATE blog_posts
   SET town_key = TRIM(REGEXP_REPLACE(LOWER(town_key), '[^[:alnum:]]+', ' ', 'g'))
 WHERE town_key IS NOT NULL
   AND town_key <> TRIM(REGEXP_REPLACE(LOWER(town_key), '[^[:alnum:]]+', ' ', 'g'));
