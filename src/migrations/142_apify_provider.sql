-- 142_apify_provider.sql — kanban B98.
--
-- Apify becomes ONE MORE enrichment source alongside Google Places / SerpAPI / Bing /
-- Google CSE — but a special one: it scrapes on a METERED, PAY-PER-USE platform, so it
-- is deliberately OFF until an admin switches it on. The adapter is registered here so
-- the Provider API Keys card can render a labelled token field, and the Data Enrichment
-- card can offer it as a pinnable provider. `src/handlers/enrichment.rs` keeps it out of
-- the auto-selection path (OPT_IN_ADAPTERS), so merely saving a token never starts a
-- paid run — the admin must PIN it.
--
-- CONSENT / SCOPE (card B98): a scrape may only GAP-FILL an existing BUSINESS listing, and
-- only ever fills EMPTY fields under the merge contract. Scraped SUPPLIERS are never
-- published from a scrape; when the internal prospecting list (card B80) exists, a scraped
-- supplier may enter it for outreach only.
--
-- Idempotent: ON CONFLICT DO NOTHING, so applying it to the live database is a no-op and a
-- re-run after a failed boot is harmless. Column list is explicit so a later column
-- addition cannot silently shift a value.

INSERT INTO public.available_providers
    (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only)
VALUES
    ('apify',
     'Apify',
     'Cloud scrapers that fill the gaps Google misses. METERED and paid per use, so it is OFF by default — add the token here, then PIN it in the Data Enrichment card to switch it on. Only fills empty fields; scraped suppliers are never published.',
     false,
     '[]'::jsonb,
     '🕸️',
     NULL,
     NULL,
     false)
ON CONFLICT (key) DO NOTHING;
