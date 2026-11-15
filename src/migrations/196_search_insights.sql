-- Search Console + GA4: the feedback loop the blog engine has been missing.
--
-- One connection per directory (the property that owns its pages). Credentials are stored the
-- same way every other provider secret is (enc:v1 ciphertext via provider_key_crypto). Daily
-- rows are kept so decay compares real windows instead of guessing, and so the weekly snapshot
-- has something to read.

CREATE TABLE IF NOT EXISTS search_insight_connections (
    directory_id UUID PRIMARY KEY REFERENCES directories(id) ON DELETE CASCADE,
    site_url TEXT NOT NULL,                 -- 'sc-domain:zaarhub.com' or 'https://zaarhub.com/'
    ga4_property_id TEXT,                   -- numeric property id, optional
    client_id TEXT NOT NULL,
    client_secret_enc TEXT NOT NULL,
    refresh_token_enc TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'connected', 'error')),
    last_error TEXT,
    last_sync_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS gsc_query_daily (
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    day DATE NOT NULL,
    query TEXT NOT NULL,
    page TEXT NOT NULL DEFAULT '',
    clicks INT NOT NULL DEFAULT 0,
    impressions INT NOT NULL DEFAULT 0,
    ctr DOUBLE PRECISION NOT NULL DEFAULT 0,
    position DOUBLE PRECISION NOT NULL DEFAULT 0,
    PRIMARY KEY (directory_id, day, query, page)
);

CREATE TABLE IF NOT EXISTS gsc_page_daily (
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    day DATE NOT NULL,
    page TEXT NOT NULL,
    clicks INT NOT NULL DEFAULT 0,
    impressions INT NOT NULL DEFAULT 0,
    ctr DOUBLE PRECISION NOT NULL DEFAULT 0,
    position DOUBLE PRECISION NOT NULL DEFAULT 0,
    PRIMARY KEY (directory_id, day, page)
);

CREATE TABLE IF NOT EXISTS ga4_page_daily (
    directory_id UUID NOT NULL REFERENCES directories(id) ON DELETE CASCADE,
    day DATE NOT NULL,
    page_path TEXT NOT NULL,
    sessions INT NOT NULL DEFAULT 0,
    views INT NOT NULL DEFAULT 0,
    PRIMARY KEY (directory_id, day, page_path)
);

CREATE INDEX IF NOT EXISTS idx_gsc_query_daily_dir_day
    ON gsc_query_daily (directory_id, day DESC);
CREATE INDEX IF NOT EXISTS idx_gsc_page_daily_dir_day
    ON gsc_page_daily (directory_id, day DESC);
CREATE INDEX IF NOT EXISTS idx_ga4_page_daily_dir_day
    ON ga4_page_daily (directory_id, day DESC);
