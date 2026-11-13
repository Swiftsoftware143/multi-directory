-- 192_retirement_is_not_overruled.sql
--
-- The companion to 191 (card t_0f61ea40). 191 let an active listing be category-less, but its
-- inference still ran on EVERY write of is_active/category_id/name/place_id — including the write
-- that retires an undecided trade. So `retire-undecided --apply` cleared 253 categories and the
-- trigger immediately re-filed 178 of them from a cached record that the retire rule had already
-- read and rejected (the trigger takes the first row matching any identity arm; the retire rule
-- takes the most recently cached match of each arm and unions the types, then judges the trade).
--
-- Two implementations of one question answered differently on 116 live listings: the sweep said
-- "no evidence", the trigger said "filed". The sweep is the authority, because it is the one that
-- applies the whole rule (name vocabulary, clusters, contradiction, and the union of identity
-- arms). The trigger's job is only to give a NEW listing a sensible default from the evidence in
-- hand — it must never overrule a decision that has already been taken.
--
-- So inference now runs only for a listing that was already undecided (a fresh insert, or an
-- unpublished row being republished). A write that clears a category is a decision, and it stands.

CREATE OR REPLACE FUNCTION businesses_require_category() RETURNS trigger AS $$
DECLARE
    city_lc  text := lower(btrim(coalesce(NEW.city, '')));
    phone10  text := right(regexp_replace(coalesce(NEW.phone, ''), '\D', '', 'g'), 10);
    host     text := lower(regexp_replace(coalesce(NEW.website, ''),
                               '^https?://(www\.)?([^/]+).*$', '\2'));
BEGIN
    IF NEW.is_active IS TRUE
       AND NEW.category_id IS NULL
       AND (TG_OP = 'INSERT' OR OLD.category_id IS NULL) THEN
        SELECT c.id INTO NEW.category_id
        FROM google_places_cache g
        CROSS JOIN LATERAL unnest(g.types) WITH ORDINALITY AS t(google_type, ord)
        JOIN business_type_category_map m ON m.google_type = t.google_type
        JOIN directory_categories c
             ON c.slug = m.category_slug
            AND (c.directory_id IS NULL OR c.directory_id = NEW.directory_id)
        WHERE (NEW.place_id IS NOT NULL AND g.place_id = NEW.place_id)
           OR (lower(g.name) = lower(NEW.name) AND city_lc <> ''
               AND lower(coalesce(g.formatted_address, '')) LIKE '%' || city_lc || '%')
           OR (lower(g.name) = lower(NEW.name) AND length(phone10) = 10
               AND phone10 = right(regexp_replace(coalesce(g.phone, ''), '\D', '', 'g'), 10))
           OR (lower(g.name) = lower(NEW.name) AND host <> ''
               AND host = lower(regexp_replace(coalesce(g.website, ''),
                                   '^https?://(www\.)?([^/]+).*$', '\2')))
        ORDER BY (NEW.place_id IS NOT NULL AND g.place_id = NEW.place_id) DESC,
                 (c.directory_id = NEW.directory_id) DESC,
                 t.ord
        LIMIT 1;
        -- Nothing decided a trade: the listing stays published as a local business.
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION businesses_require_category() IS
    'Truth rule (t_0f61ea40): a NEW listing gets a default trade from identity-grade cached '
    'evidence (place id, or name + city/phone/host). Nothing decided => category NULL and the '
    'listing stays published as a local business. A write that clears a category is a decision '
    'taken by the retire rule and is never overruled here (192).';
