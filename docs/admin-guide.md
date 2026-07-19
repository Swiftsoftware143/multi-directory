# Admin Guide — Multi-Directory Platform

## Admin Pages

| Page | Path | Description |
|---|---|---|
| Dashboard | `/` | Stats overview — listings, articles, blog posts |
| Directories | `/directories` | Create/manage directories. Feature toggles per directory |
| Listings | `/listings` | Browse all business listings across directories |
| Categories | `/categories` | Manage category taxonomy |
| SEO | `/seo` | SEO settings, sitemaps, meta defaults |
| Blog | `/blog` | Write and manage blog posts |
| Deals | `/deals` | Create/manage deals, view redemptions |
| API Keys | `/apikeys` | Configure third-party API keys (Google Places, BrightLocal, Yext, Uberall, etc.) |
| Settings | `/settings` | Tenant name, password, SMTP config |
| Monetization | `/monetization` | Plan tiers, subscriptions, sponsored articles, ad zones |
| Analytics | `/analytics` | View tracking data and engagement metrics |
| Integrations | `/integrations` | Core Swift, external CRM connections |
| Workflows | `/workflows` | Automation workflow templates |
| CRM | `/crm` | Contact management, pipelines, deals |
| Email | `/email` | Send emails to directory businesses |
| Branding | `/branding` | Customize directory look and feel |
| Trap Doors | `/trapdoors` | Build hyper-niche SEO pages |
| SEO Articles | `/busarticles` | Generate AI-optimized articles for businesses |

## Feature Toggles per Directory

Each directory has a `feature_config` JSONB field that controls what features are available:

```json
{
  "deals": true,
  "blogging": true,
  "community_posts": true,
  "b2b_marketplace": false,
  "visitor_accounts": true,
  "gamification": false
}
```

Toggle these in the Directory Settings panel.

## API Keys (Settings > API Keys)

Configure provider keys for data enrichment and scraping:

| Provider | Purpose |
|---|---|
| Google Places | Business autocomplete, details, reviews, photos |
| BrightLocal | Local business data, listings, citations |
| Yext | Business listings management |
| Uberall | Local presence management |
| Mailgun / SendGrid | Transactional email |
| OpenAI / DeepSeek | AI content generation |
| Telnyx | SMS/Voice |

## Scraper Engine

The scraper engine (`/api/v1/scraper/`) supports multiple sources:

- **Google Places** — fully integrated. Run a search by location + keyword, results stream through the pipeline.
- **BrightLocal, Yext, Uberall** — provider framework ready. Add your API key in Settings > API Keys.
- **Nextdoor, Chamber, YellowPages** — scraper framework ready. Add provider key in Settings.

Results from all scrapers flow through `/api/v1/pipeline/ingest` for dedup, merge, and enrichment.

## B2B Marketplace

The B2B marketplace distinguishes suppliers (distributors, wholesalers, farms, associations) from local businesses. Suppliers appear in their own search tab with product listings.

- `/api/v1/b2b/products` — search and manage supplier products
- `/api/v1/b2b/suppliers` — list all supplier-type businesses
- Business type is set per-listing: `local`, `supplier`, `distributor`, `wholesaler`, `farm`, `association`

## Deal Management

| Endpoint | Description |
|---|---|
| `/api/v1/deals` | CRUD operations |
| `/api/v1/deals/:id/redeem` | Generate redemption code |
| `/api/v1/deals/:id/redemptions` | View redemption history |
| `/api/v1/deals/redemptions/:rid/use` | Mark code as used |
| `/api/v1/deals/redemptions/expire` | Expire old codes |
| `/api/v1/deals/redemptions/code/:code` | Look up by code |

## Pricing Engine

Admin-only endpoints at `/api/v1/pricing/`:

| Endpoint | Description |
|---|---|
| `/pricing/services` | List/update service prices |
| `/pricing/bundles` | Create/manage pricing bundles |
| `/pricing/grandfather` | Set grandfathered pricing per business |
| `/pricing/public` | Public-facing standard pricing |
