-- 140_fresh_install_seed.sql — kanban B85 (handover readiness).
--
-- WHY THIS EXISTS. `000_baseline_live_schema.sql` is the live CATALOG: schema only, ZERO rows.
-- A buyer applying it to an empty database therefore got
--   * no `swiftsoftware` tenant — every platform-scoped row hangs off it, so `create-admin`
--     refused outright and there was nowhere to put platform configuration, and
--   * an EMPTY provider catalogue — the admin panel's "Provider API Keys" section rendered
--     nothing, so a capability that is explicitly operator-configurable (nothing hardwired)
--     could not be configured at all.
-- This file ships the minimum reference rows a fresh install needs to be OPERABLE. Nothing here
-- is tenant content: per-directory branding, categories, businesses, loyalty programmes, legal
-- pages and email templates are created by the operator in the admin panel.
--
-- Idempotent three ways: ON CONFLICT DO NOTHING everywhere, so applying it to the LIVE database
-- is a no-op, and re-running a failed boot is harmless. Column lists are explicit so a later
-- column addition cannot silently shift a value.
--
-- The tenant id is the canonical platform id that `src/system_tenant.rs` references; the row is
-- resolved BY SLUG at runtime, never by copying this literal (RULES Part 1).

-- 1. The platform tenant (migration 001 seeded it on the original host; the baseline carries no rows).
INSERT INTO tenants (id, name, slug, is_active)
VALUES ('00000000-0000-0000-0000-000000000001', 'SwiftSoftware', 'swiftsoftware', true)
ON CONFLICT DO NOTHING;

-- 2. The provider catalogue the admin panel renders. Values are PRESETS (names, labels, help
--    text); every key itself lives in provider_keys, encrypted at rest, entered per directory.
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('sendiio', 'Sendiio', 'Email/SMS campaign delivery', false, '[]', 'mail', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('telnyx', 'Telnyx', 'SMS/Voice API', false, '[]', 'phone', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('nexweave', 'Nexweave', 'Personalized video/image generation', false, '[]', 'video', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('sam_gov', 'SAM.gov', 'Federal contracting opportunities', false, '[]', 'shield', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('usaspending', 'USAspending', 'Federal spending data', false, '[]', 'dollar-sign', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('letterman', 'Letterman', 'Newsletter content delivery', false, '[]', 'newspaper', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('stripe', 'Stripe', 'Card payments, subscriptions, invoices and payouts', false, '[]', '💳', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('twilio', 'Twilio', 'SMS + voice messaging', false, '[]', '📞', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('openai', 'OpenAI', 'GPT models for AI copy, summaries and drafts', false, '[]', '🤖', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('anthropic', 'Anthropic (Claude)', 'Claude models for long-form AI content', false, '[]', '🧠', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('serpapi', 'SerpAPI', 'Google/Bing SERP scraping for rank tracking', false, '[]', '🔎', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('google_cse', 'Google Custom Search', 'Programmable Search Engine JSON API', false, '[{"key": "cx", "label": "Search Engine ID (cx)"}]', '🔍', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('bing', 'Bing Web Search', 'Bing Web Search API (Azure Cognitive Services)', false, '[]', '🅱️', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('mintbird', 'Mintbird', 'Cold-email outreach + warmup automation', false, '[]', '📧', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('groovesell', 'GrooveSell', 'Checkout, affiliate and upsell platform', false, '[]', '🛒', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('brightlocal', 'BrightLocal', 'Citation building + local rank grid', false, '[]', '⭐', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('yext', 'Yext', 'Listings sync + knowledge graph', false, '[]', '🗺️', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('uberall', 'Uberall', 'Listings management + local SEO', false, '[]', '📍', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('mailgun', 'Mailgun', 'Transactional + campaign email', false, '[]', '✉️', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('sendgrid', 'SendGrid', 'Transactional + campaign email', false, '[]', '✉️', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('deepseek', 'DeepSeek', 'DeepSeek LLM API', false, '[]', '🧠', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('coreswift', 'CoreSwift CRM', 'Push leads into CoreSwift CRM', false, '[]', 'crm', NULL, NULL, false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('dataforseo', 'DataForSEO', 'Keyword ideas and search volume for the Blog/QA keyword tool. Uses HTTP Basic auth, so it needs BOTH your DataForSEO account email (the login) and the API password from the DataForSEO dashboard.', true, '[]', '🔑', 'DataForSEO login (account email)', 'Your DataForSEO account email. The API key field above holds the matching API password from dashboard.dataforseo.com → API Access.', false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('yelp', 'Yelp', 'Business search and details from the Yelp Fusion API, used to find and import businesses into a directory.', false, '[]', '🍽️', 'Yelp API key', 'From the Yelp Fusion dashboard (yelp.com/developers → Manage App → API Key). Create the app inside the same Yelp account that owns the listings.', false) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('google_maps', 'Google Maps (browser)', 'Draws the interactive map on a business page, in the visitor''s browser. BROWSER-ONLY: the key is visible in the page source, so restrict it by HTTP referrer to your own domains. It cannot do server-side search — that is the Google Places key.', false, '[]', '🗺', 'Google Maps JavaScript API key', 'BROWSER-ONLY — travels to the visitor''s browser. In Google Cloud restrict this key by HTTP referrer to your own domains (e.g. https://zaarhub.com/*). Never restrict it by server IP, never use it for server-side calls, and never reuse one "master" key for both jobs.', true) ON CONFLICT (key) DO NOTHING;
INSERT INTO public.available_providers (key, name, description, requires_base_url, requires_metadata, icon, field_label, field_help, browser_only) VALUES ('google_places', 'Google Places', 'Local business data API — listings, reviews, citations', false, '[]', '🗺️', NULL, 'SERVER-SIDE ONLY — business search, discovery and enrichment. Restrict this key in Google Cloud by server IP address; never by website referrer. Already set at network level, so a city directory with no key of its own inherits it.', false) ON CONFLICT (key) DO NOTHING;
