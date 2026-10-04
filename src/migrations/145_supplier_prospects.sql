-- 145_supplier_prospects.sql — card B80: internal supplier PROSPECTING.
--
-- David (2026-09-30): "I don't pre-populate the suppliers ... But I do want to have a way to
-- search for them so I can reach out to them." So a prospect is an INTERNAL record with no
-- public presence, and it must never leak into the public B2B marketplace or supplier directory.
--
-- Design (reuses the card-B81 lifecycle instead of inventing a parallel system):
--   * the prospect RECORD is an existing `businesses` row with `status='prospect'` (and
--     `is_active=false`), exactly like every other unpublished record — so it is excluded from
--     every public surface (search filters `is_active`; detail/sitemap/SEO filter the status)
--     by construction, with no second record and no parallel listing table;
--   * this file holds only the OUTREACH state that is not part of a business: the prospect's
--     pipeline status + notes + source, and an append-only outreach log.
--
-- Nothing here is ever served publicly. Converting a prospect to a real supplier is an explicit
-- action that PROMOTES the same `businesses` row (status -> 'active'), never a duplicate.

CREATE TABLE IF NOT EXISTS public.supplier_prospects (
    business_id uuid PRIMARY KEY REFERENCES public.businesses(id) ON DELETE CASCADE,
    status      text NOT NULL DEFAULT 'new',
    notes       text NOT NULL DEFAULT '',
    source      text NOT NULL DEFAULT '',
    created_by  uuid,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Pipeline status. Added as a named constraint so the file is idempotent on re-run.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'supplier_prospects_status_check'
    ) THEN
        ALTER TABLE public.supplier_prospects
            ADD CONSTRAINT supplier_prospects_status_check
            CHECK (status = ANY (ARRAY['new', 'contacted', 'replied', 'interested',
                                       'onboarded', 'declined']));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_supplier_prospects_status
    ON public.supplier_prospects (status);

CREATE TABLE IF NOT EXISTS public.supplier_prospect_outreach (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id uuid NOT NULL REFERENCES public.businesses(id) ON DELETE CASCADE,
    channel     text NOT NULL DEFAULT 'note',
    note        text NOT NULL DEFAULT '',
    created_by  uuid,
    created_at  timestamptz NOT NULL DEFAULT now()
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'supplier_prospect_outreach_channel_check'
    ) THEN
        ALTER TABLE public.supplier_prospect_outreach
            ADD CONSTRAINT supplier_prospect_outreach_channel_check
            CHECK (channel = ANY (ARRAY['note', 'email', 'phone', 'other']));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_supplier_prospect_outreach_business
    ON public.supplier_prospect_outreach (business_id, created_at DESC);
