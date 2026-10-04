-- 149_directory_industry.sql — a directory DECLARES the industry (niche) it is built on (card B115).
--
-- David's strategy (2026-10-03): "an inch wide and a mile deep — not a directory for Florida
-- Businesses but for something like Organic Farms and Wholesalers in Central Florida. Make the
-- product reflect it: niche-able directory structure, categories/industries that support deep
-- specialisation … Keep it configurable — a buyer points it at their own niche."
--
-- The industry taxonomy (`template_categories`, migration 108) already exists and is managed
-- entirely from the admin panel's Industries section — but no directory could DECLARE which
-- industry it is. `directories.template` chooses the LAYOUT (farm / restaurant / …); this column
-- records the directory's NICHE, so "Organic Farms & Wholesalers in Central Florida" becomes a
-- first-class, queryable property of the directory (and of its SEO), settable by a non-technical
-- buyer from the panel.
--
-- ON DELETE SET NULL: a directory outlives its industry row (unpublishing/removing an industry
-- must never delete a directory). `template_categories.slug` is UNIQUE (constraint
-- template_categories_slug_key), so it is a valid FK target.
--
-- Idempotent: safe on a fresh install and safe to re-run on a live DB.

ALTER TABLE public.directories ADD COLUMN IF NOT EXISTS industry_slug text;

-- Clear any pre-existing / hand-set value that does not resolve to a catalogue row BEFORE the
-- constraint is validated, so VALIDATE can never fail on legacy data or drift.
UPDATE public.directories d
   SET industry_slug = NULL
 WHERE d.industry_slug IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM public.template_categories t WHERE t.slug = d.industry_slug);

ALTER TABLE public.directories DROP CONSTRAINT IF EXISTS directories_industry_slug_fkey;

ALTER TABLE public.directories
  ADD CONSTRAINT directories_industry_slug_fkey
  FOREIGN KEY (industry_slug) REFERENCES public.template_categories(slug)
  ON DELETE SET NULL NOT VALID;

ALTER TABLE public.directories VALIDATE CONSTRAINT directories_industry_slug_fkey;

CREATE INDEX IF NOT EXISTS idx_directories_industry_slug ON public.directories (industry_slug);
