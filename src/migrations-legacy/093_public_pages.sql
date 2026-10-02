-- 093: public_pages — the table this feature's handlers have always queried.
--
-- src/handlers/public.rs has served /public-pages routes since the feature was written, but the
-- table was never created, so every call answered 500 "relation \"public_pages\" does not exist".
-- The column list below is taken verbatim from the handler's own SELECT/INSERT statements, so the
-- schema matches what the code already expects (a deals-like promotional record with its own
-- public_page_price / public_page_type fields).

CREATE TABLE IF NOT EXISTS public_pages (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    title               text NOT NULL,
    description         text,
    original_price      numeric(10,2),
    public_page_price   numeric(10,2),
    discount_percent    numeric(5,2),
    currency            text DEFAULT 'USD',
    image_url           text,
    terms               text,
    redemption_limit    integer,
    redemption_count    integer NOT NULL DEFAULT 0,
    status              text NOT NULL DEFAULT 'active',
    directory_id        uuid REFERENCES directories(id) ON DELETE CASCADE,
    business_id         uuid REFERENCES businesses(id) ON DELETE SET NULL,
    start_date          timestamptz,
    end_date            timestamptz,
    featured            boolean NOT NULL DEFAULT false,
    public_page_type    text,
    coupon_code         text,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_public_pages_directory_id ON public_pages(directory_id);
CREATE INDEX IF NOT EXISTS idx_public_pages_business_id ON public_pages(business_id);
CREATE INDEX IF NOT EXISTS idx_public_pages_status ON public_pages(status);
CREATE INDEX IF NOT EXISTS idx_public_pages_featured ON public_pages(featured) WHERE featured = true;
CREATE INDEX IF NOT EXISTS idx_public_pages_created_at ON public_pages(created_at DESC);
