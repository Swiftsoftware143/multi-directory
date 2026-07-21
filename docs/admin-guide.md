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

## Cross-Platform Tag Sync

### Overview
When a user signs up or submits a survey in MultiDirectory, their role and preference tags propagate to IncentiveSwift (loyalty engine) and CoreSwift (contact CRM). This enables campaign eligibility filtering, newsletter segmentation, and CRM contact management.

### Tag Naming Convention (ZaarHub)
Newsletter tags follow the format `{city-code}-zh-newsletter`:

| City | Code | Tag |
|---|---|---|
| Apopka | ap | `ap-zh-newsletter` |
| Boca Raton | br | `br-zh-newsletter` |
| Hollywood | hw | `hw-zh-newsletter` |
| Lake Nona | ln | `ln-zh-newsletter` |
| Palm Bay | pb | `pb-zh-newsletter` |
| Palm Coast | pc | `pc-zh-newsletter` |
| Pompano Beach | pp | `pp-zh-newsletter` |
| St. Cloud | sc | `sc-zh-newsletter` |
| St. Petersburg | sp | `sp-zh-newsletter` |
| Winter Garden | wg | `wg-zh-newsletter` |

### How It Works
1. **Survey submission triggers sync** — `public_submit_survey` processes visitor answers, generates granular tags (e.g. `Co-op`, `Farmer`) and newsletter tags (`pc-zh-newsletter`), then calls `fire_tag_sync()`.
2. **Standalone newsletter signup triggers sync** — `POST /directories/:slug/subscribers` adds the `Subscriber` + `{code}-zh-newsletter` tags.
3. **Supplier classification** — Dropdown values map to granular tags:
   - "Farmer / Grower" → `Farmer`
   - "Wholesale Distributor" → `Wholesale Distributor`
   - "Manufacturer / Factory Producer" → `Manufacturer`
   - "Trade Association / Co-op" → `Co-op`
   - "Food Hub / Aggregator" → `Food Hub`
   - "Artisan / Specialty Craft Producer" → `Artisan`
   - "Importer / Exporter" → `Importer / Exporter`
   - "Logistics & Freight Provider" → `Logistics Provider`
   - "Raw Material Supplier" → `Raw Material Supplier`
4. **IncentiveSwift** — Contact created/updated with tags via `/api/v1/loyalty/external/tag-contact`.
5. **CoreSwift** — Contact created via `push_newsletter_signup` with city tag from `_city_tags` table, plus tag sync via `/api/v1/webhooks/cross-app/tag-sync`.

### Manual Testing
```bash
# Test IncentiveSwift endpoint directly
curl -X POST http://localhost:8083/api/v1/loyalty/external/tag-contact \
  -H "Content-Type: application/json" \
  -d '{
    "email": "test@example.com",
    "first_name": "Test",
    "last_name": "User",
    "tags": ["Subscriber", "pc-zh-newsletter"],
    "source": "test"
  }'

# Test newsletter signup API
curl -X POST http://localhost:3001/api/v1/directories/palm-coast/subscribers \
  -H "Content-Type: application/json" \
  -d '{"email": "test@example.com", "name": "Test User"}'

# Check CoreSwift tags
docker exec swift-postgres-1 psql -U swift -d coreswift \
  -c "SELECT c.email, t.name FROM tag_assignments ta JOIN tags t ON t.id = ta.tag_id JOIN contacts c ON c.id = ta.entity_id WHERE c.email = 'test@example.com';"
```

### Tags Created on All Platforms
`Business`, `Association`, `Farm`, `Wholesaler`, `Distributor`, `Sponsor`, `Subscriber`, `Customer`, `Supplier`, and all granular supplier tags + city newsletter tags.

### Known Limitations
- CoreSwift list-by-name requires JWT — use `coreswift_list_id` (resolved UUID) for list membership
- Tags are comma-separated in IncentiveSwift `notes2` column (no dedicated join table)
