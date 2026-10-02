-- Migration 084: discovery queue (round 5, T3).
--
-- David's workflow: pick a category -> search -> the results land in a queue that
-- survives a reload -> franchises are auto-flagged and excluded -> already-listed
-- businesses are marked -> he ticks the rows he wants and bulk-adds them.
--
-- One queue per directory. `place_id` is Google's id when we have one (a Google
-- type search returns it); when it is absent we fall back to name+address, which
-- is also how we detect "already in the directory" (the businesses table has no
-- place_id column).

CREATE TABLE IF NOT EXISTS discovery_queue (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    directory_id       uuid NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    place_id           text,
    name               text NOT NULL,
    address            text,
    city               text,
    phone              text,
    website            text,
    rating             double precision DEFAULT 0,
    review_count       integer DEFAULT 0,
    types              text[] NOT NULL DEFAULT '{}',
    mapped_category_id uuid,
    mapped_category    text,
    is_franchise       boolean NOT NULL DEFAULT false,
    is_duplicate       boolean NOT NULL DEFAULT false,
    selected           boolean NOT NULL DEFAULT false,
    status             text NOT NULL DEFAULT 'queued',
    raw                jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_discovery_queue_dir
    ON discovery_queue (directory_id, status);

-- Dedup guards: same Google place once per directory...
CREATE UNIQUE INDEX IF NOT EXISTS discovery_queue_place_uniq
    ON discovery_queue (directory_id, place_id) WHERE place_id IS NOT NULL;
-- ...and same name+address once per directory (case-insensitive).
CREATE UNIQUE INDEX IF NOT EXISTS discovery_queue_nameaddr_uniq
    ON discovery_queue (directory_id, lower(name), lower(COALESCE(address, '')));
