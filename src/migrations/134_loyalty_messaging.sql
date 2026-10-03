-- 134_loyalty_messaging.sql — card B92: admin-configurable ZaarCash loyalty messaging.
--
-- David (2026-10-01): the loyalty programme is the differentiator against Nextdoor / Angie's List /
-- Thumbtack, so "there should be some kind of verbiage on the homepage as well as each city
-- homepage mentioning it". Two rules from the card:
--   (1) NOTHING HARDCODED — on/off + the copy + the CTA target are editable per NETWORK and per
--       standalone DIRECTORY, because a buyer's programme is their own. Resolution: directory ->
--       network -> built-in defaults (same rule as the brand/config resolution used elsewhere).
--   (2) HONEST STATE — the served copy must reflect whether earning is ACTUALLY switched on. The
--       earn state is computed at read time from loyalty_programs (never stored here), so the
--       section flips to full earn messaging automatically the moment a rate is set, and shows a
--       truthful "how it works" state while nothing can be earned.
--
-- Exactly one owner row per scope, mirroring homepage_sections.homepage_owner_check.
CREATE TABLE IF NOT EXISTS public.loyalty_messaging (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    network_id uuid,
    directory_id uuid,
    enabled boolean DEFAULT true NOT NULL,
    headline text,
    subheadline text,
    cta_label character varying(80),
    cta_url character varying(500),
    show_earn_claims boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now()
);

DO $md_bl$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_messaging_pkey' AND conrelid = 'public.loyalty_messaging'::regclass
    ) THEN
        ALTER TABLE public.loyalty_messaging ADD CONSTRAINT loyalty_messaging_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_messaging_owner_check' AND conrelid = 'public.loyalty_messaging'::regclass
    ) THEN
        ALTER TABLE public.loyalty_messaging ADD CONSTRAINT loyalty_messaging_owner_check
            CHECK ((network_id IS NOT NULL AND directory_id IS NULL)
                OR (network_id IS NULL AND directory_id IS NOT NULL));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_messaging_network_id_fkey' AND conrelid = 'public.loyalty_messaging'::regclass
    ) THEN
        ALTER TABLE public.loyalty_messaging ADD CONSTRAINT loyalty_messaging_network_id_fkey
            FOREIGN KEY (network_id) REFERENCES public.networks(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'loyalty_messaging_directory_id_fkey' AND conrelid = 'public.loyalty_messaging'::regclass
    ) THEN
        ALTER TABLE public.loyalty_messaging ADD CONSTRAINT loyalty_messaging_directory_id_fkey
            FOREIGN KEY (directory_id) REFERENCES public.directories(id) ON DELETE CASCADE;
    END IF;
END
$md_bl$;

CREATE UNIQUE INDEX IF NOT EXISTS loyalty_messaging_network_uniq
    ON public.loyalty_messaging (network_id) WHERE network_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS loyalty_messaging_directory_uniq
    ON public.loyalty_messaging (directory_id) WHERE directory_id IS NOT NULL;

-- Seed the network-scope row for ZaarHub (the live network that owns all 10 directories). Copy is
-- deliberately NEUTRAL and does not promise per-job earnings (card B93 gating); the served
-- subheadline is replaced by the honest line while no rate is set. Idempotent: only when absent.
INSERT INTO public.loyalty_messaging (network_id, enabled, headline, subheadline, cta_label, cta_url, show_earn_claims)
SELECT n.id, true,
       'Earn ZaarCash when you shop local',
       'One balance the whole network accepts — support participating local businesses, then spend your ZaarCash in any city, not just where you earned it.',
       'See how it works',
       '/visitor',
       true
FROM public.networks n
WHERE n.slug = 'zaarhub'
  AND NOT EXISTS (
      SELECT 1 FROM public.loyalty_messaging m WHERE m.network_id = n.id
  );
