# Admin Guide — ZaarHub Multi-Directory Platform

## Quick Reference

| Page | Path | What it does |
|---|---|---|
| Dashboard | `/` | Overview of all directories |
| Directories | `/directories` | Create/manage directories, set feature toggles |
| Listings | `/listings` | All business listings across directories |
| Categories | `/categories` | Category management |
| API Keys | `/apikeys` | Configure keys for Google Places, BrightLocal, etc. |
| Monetization | `/monetization` | Plan tiers, subscriptions, grandfathered pricing |
| Deals | `/deals` | Manage deals, view redemptions |

## Feature Toggles (per directory)

Controls what features a directory has:
```json
{"deals": true, "blogging": true, "community_posts": true, "b2b_marketplace": false, "visitor_accounts": true}
```

## API Keys

Required for data import and AI features:
- **Google Places** — business search and import
- **BrightLocal / Yext / Uberall** — data enrichment
- **OpenAI / DeepSeek** — AI article generation
- **Mailgun / SendGrid** — email delivery

## Data Import

Single tool at Settings → API Keys. Select source, type (Businesses or Suppliers), location, keyword. Results deduplicate and merge automatically.

## Deal Management

Deals have visual templates (classic, modern, bold, minimal), countdown timers, customizable CTA buttons and colors, gallery images, and rotation scheduling. Deal pages at `zaarhub.com/deals/{id}`.

## Supplier Portal

Separate back office at `zaarhub.com/supplier/` for distributors, wholesalers, farms, and associations. See the Supplier Guide.

## Guides

All guides available at `zaarhub.com/guides/`.
