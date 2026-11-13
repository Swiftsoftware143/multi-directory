-- 191_locals_may_have_no_trade.sql
--
-- THE TRUTH RULE (card t_0f61ea40).
--
-- Migration 189 assumed the opposite of what is true: "A category-less page is worse than an
-- unpublished listing", so it made an active listing with `category_id IS NULL` unreachable —
-- the guard resolved a category from the cached Places record, and when nothing resolved it set
-- `is_active := false`. Two consequences, both live:
--
--   1. Every one of the 4,113 listings carries a category, so a trade can only ever be WRONG,
--      never absent. 253 of them are published under a trade that their own provider record and
--      name do not name (e.g. RealManage Winter Garden, a property manager published as
--      "Landscaping in Winter Garden"), and the guard is what made clearing one impossible: the
--      only way to say "no trade" was to unpublish the business.
--   2. The guard's identity test was `lower(g.name) = lower(NEW.name)` — the exact cross-city
--      borrowing bug card t_037e2e4a fixed in Rust: two businesses with the same name in
--      different cities, and the type of one files the other.
--
-- A page that says "X is a local business in <city>" is a legitimate, indexable, linkable page —
-- the copy composer has always rendered it, and a listing with no decided trade is a listing we
-- cannot prove a trade for. Publishing a false trade to avoid that state is the worse outcome.
--
-- So the guard now:
--   * still FILES a listing it can prove a trade for, but only on identity-grade evidence —
--     the exact place id, or the same name AND (the listing's city in the cached address, or the
--     same phone, or the same website host). Name alone is not an identity.
--   * never unpublishes. When nothing decides a trade, the listing stays live and category-less,
--     and every surface renders the local-business form (title, meta, H1, body, breadcrumb,
--     chips, "More <trade>", `/<city>/<trade>`) because they all read `category_id`.

CREATE OR REPLACE FUNCTION businesses_require_category() RETURNS trigger AS $$
DECLARE
    city_lc  text := lower(btrim(coalesce(NEW.city, '')));
    phone10  text := right(regexp_replace(coalesce(NEW.phone, ''), '\D', '', 'g'), 10);
    host     text := lower(regexp_replace(coalesce(NEW.website, ''),
                               '^https?://(www\.)?([^/]+).*$', '\2'));
BEGIN
    IF NEW.is_active IS TRUE AND NEW.category_id IS NULL THEN
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
        -- No `is_active := false` any more: nothing decided a trade, so the listing is a local
        -- business. It stays published and renders that way everywhere.
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS businesses_require_category_trg ON businesses;
CREATE TRIGGER businesses_require_category_trg
    BEFORE INSERT OR UPDATE OF is_active, category_id, name, place_id ON businesses
    FOR EACH ROW EXECUTE FUNCTION businesses_require_category();

COMMENT ON FUNCTION businesses_require_category() IS
    'Truth rule (t_0f61ea40): file a listing under a trade only on identity-grade cached '
    'evidence (place id, or name + city/phone/host). Nothing decided => category stays NULL and '
    'the listing stays published as a local business. Supersedes 189''s unpublish-on-no-category.';
