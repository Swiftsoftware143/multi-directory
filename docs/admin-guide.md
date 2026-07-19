# Admin Guide — ZaarHub Multi-Directory Platform

## Overview
The admin panel manages all directories, listings, suppliers, deals, and system configuration. Access at `zaarhub.com` with admin credentials.

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
| API Keys | `/apikeys` | Configure third-party API keys for scrapers and integrations |
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

Each directory has a `feature_config` JSONB field that controls features:

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

Toggle these in the Directory Settings panel. Use this to create simple directories (e.g., a Farmer's Directory with just listings) vs full community directories.

## API Keys (Settings > API Keys)

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

The scraper engine (`/api/v1/scraper/`) ingests business data from multiple sources:

- **Google Places** — fully integrated. Run a search by location + keyword, results stream through the pipeline.
- **BrightLocal, Yext, Uberall** — provider framework ready. Add your API key in Settings > API Keys.
- **Nextdoor, Chamber, YellowPages** — scraper framework ready.

All results flow through `/api/v1/pipeline/ingest` for dedup, merge, and enrichment.

## B2B Marketplace

The B2B marketplace distinguishes suppliers from local businesses. Suppliers appear in their own search tab with product listings. Business types:

- `local` — Standard local business (restaurant, plumber, dentist, etc.)
- `supplier` — General supplier
- `distributor` — Product distributor
- `wholesaler` — Wholesale goods provider
- `farm` — Farm / agricultural producer
- `association` — Trade association

Endpoints: `/api/v1/b2b/products`, `/api/v1/b2b/suppliers`

## Supplier Portal

Suppliers have their own back office at `zaarhub.com/supplier/`. See the [Supplier Guide](supplier-guide.md) for details.

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

## Visitor Accounts

Visitors can create free accounts to:
- Save favorite businesses
- Claim and redeem deals
- Leave reviews
- Subscribe to city newsletters
- Participate in community engagement

## Networks

Directories can belong to a network, inheriting shared feature flags. Standalone directories use their own settings.
