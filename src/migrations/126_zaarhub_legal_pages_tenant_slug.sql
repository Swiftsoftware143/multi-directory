-- 126_zaarhub_legal_pages_tenant_slug.sql — B95
--
-- Legal pages must be owned PER TENANT. The old constraint
--   zaarhub_legal_pages_slug_key UNIQUE (slug)
-- made a slug global across the whole platform: once ZaarHub (the system tenant)
-- owned 'terms', NO other directory/tenant could ever create its own Terms,
-- Privacy Policy or contest-rules page — the insert died on the unique index.
-- It also meant a slug-only read could render another tenant's legal text.
--
-- Replace it with UNIQUE (tenant_id, slug) so each tenant owns its own copy of a
-- slug. Every read path is tenant-scoped in the handlers (zaarhub_admin,
-- zaarhub_ssr, b2b_ssr, zaarhub_seo), so this table can no longer leak across
-- tenants. Idempotent: safe on a fresh install (the baseline creates the old
-- constraint) and on a live DB that already has the new one.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'zaarhub_legal_pages_slug_key'
          AND conrelid = 'public.zaarhub_legal_pages'::regclass
    ) THEN
        ALTER TABLE public.zaarhub_legal_pages
            DROP CONSTRAINT zaarhub_legal_pages_slug_key;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'zaarhub_legal_pages_tenant_slug_key'
          AND conrelid = 'public.zaarhub_legal_pages'::regclass
    ) THEN
        ALTER TABLE public.zaarhub_legal_pages
            ADD CONSTRAINT zaarhub_legal_pages_tenant_slug_key
            UNIQUE (tenant_id, slug);
    END IF;
END $$;
