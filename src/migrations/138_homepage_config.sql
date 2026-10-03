-- 138_homepage_config.sql — card B86: admin-configurable HOMEPAGE per network (and per standalone
-- directory) — "a city is not the home".
--
-- David (2026-09-30): "make sure that in the admin panel there is a place where you can SET THE
-- HOMEPAGE. Because the homepage isn't part of the directory if the directory is a city. But the
-- network still should have a home page. So the admin should be able to configure it."
--
-- The homepage belongs to the NETWORK or to a STANDALONE directory, never to a city. This table
-- stores exactly one row per owning scope (mirroring homepage_sections.homepage_owner_check), and
-- every field is optional with sensible defaults so an unconfigured network still renders a sane
-- homepage. Nothing here is hardcoded into a buyer's site: ZaarHub's home is one configuration of
-- it and a sold directory sets its own from the panel.
CREATE TABLE IF NOT EXISTS public.homepage_config (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    directory_id uuid,
    enabled boolean DEFAULT true NOT NULL,
    -- 'network' (serve the network's own home) or 'city' (feature one of its cities as the front page)
    home_surface character varying(20) DEFAULT 'network' NOT NULL,
    home_city_slug character varying(120),
    announcement_text text,
    announcement_cta_text character varying(120),
    announcement_cta_url character varying(500),
    hero_headline text,
    hero_subheadline text,
    hero_image_url text,
    hero_cta_text character varying(120),
    hero_cta_url character varying(500),
    -- Ordered list of slugs to highlight on the home; empty = every city in the network.
    featured_city_slugs text[] DEFAULT '{}'::text[] NOT NULL,
    -- Ordered list of {"key": "<section>", "enabled": <bool>} — which blocks render and in what order.
    sections jsonb DEFAULT '[]'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_config_pkey' AND conrelid = 'public.homepage_config'::regclass
    ) THEN
        ALTER TABLE public.homepage_config ADD CONSTRAINT homepage_config_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_config_owner_check' AND conrelid = 'public.homepage_config'::regclass
    ) THEN
        ALTER TABLE public.homepage_config ADD CONSTRAINT homepage_config_owner_check
            CHECK ((network_id IS NOT NULL AND directory_id IS NULL)
                OR (network_id IS NULL AND directory_id IS NOT NULL));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_config_surface_check' AND conrelid = 'public.homepage_config'::regclass
    ) THEN
        ALTER TABLE public.homepage_config ADD CONSTRAINT homepage_config_surface_check
            CHECK (home_surface IN ('network', 'city'));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_config_network_id_fkey' AND conrelid = 'public.homepage_config'::regclass
    ) THEN
        ALTER TABLE public.homepage_config ADD CONSTRAINT homepage_config_network_id_fkey
            FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'homepage_config_directory_id_fkey' AND conrelid = 'public.homepage_config'::regclass
    ) THEN
        ALTER TABLE public.homepage_config ADD CONSTRAINT homepage_config_directory_id_fkey
            FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

CREATE UNIQUE INDEX IF NOT EXISTS homepage_config_network_uniq
    ON public.homepage_config (network_id) WHERE network_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS homepage_config_directory_uniq
    ON public.homepage_config (directory_id) WHERE directory_id IS NOT NULL;

-- Seed the ZaarHub network row so the admin console has a real record to edit rather than an empty
-- form; every field is left NULL/empty so the served homepage keeps its built-in defaults until the
-- admin changes something. Idempotent: only when absent.
INSERT INTO public.homepage_config (network_id, enabled, home_surface, featured_city_slugs, sections)
SELECT n.id, true, 'network', '{}'::text[], '[]'::jsonb
FROM public.networks n
WHERE n.slug = 'zaarhub'
  AND NOT EXISTS (
      SELECT 1 FROM public.homepage_config c WHERE c.network_id = n.id
  );
