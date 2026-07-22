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
{"deals": true, "blogging": true, "community_posts": true, "b2b_marketplace": false, "visitor_accounts": true, "onboarding_survey": true}
```

## API Keys

Required for data import and AI features:
- **Google Places** — business search and import
- **BrightLocal / Yext / Uberall** — data enrichment
- **OpenAI / DeepSeek** — AI article generation
- **Mailgun / SendGrid** — email delivery

## Data Import

Single tool at Settings → API Keys. Select source, type (Businesses or Suppliers), location, keyword. Results deduplicate and merge automatically.

## City Requests

**Location:** Admin Sidebar → City Requests (🏗️ icon)

Visitors can submit city name + email to request their city be added. The admin panel shows all requests ranked by vote count:
- **Pending** — gray badge, has "Mark Added" action button
- **Added** — green badge, shows processed date
- Click "Mark Added" on any pending request to mark the city as launched
- Useful for prioritizing which new markets to open

API endpoints:
- `GET /api/v1/admin/directories/:id/city-requests` — list all requests for a directory
- `POST /api/v1/admin/directories/:id/city-requests/:request_id/mark-added` — mark as added

## Visitor Bookmarks / Saved Places

Visitors can bookmark businesses (heart icon on listing pages).
- Bookmark count displayed on each business listing (`GET /api/v1/bookmarks/count/:business_id`)
- Visitors view/manage bookmarks at `/saved-places`
- Bookmarks require a visitor account (free signup)
- No admin action needed — fully visitor-driven

## Blogging System

**Location:** Admin Sidebar → Blogging (📝 icon)

The blogging system now has a tabbed accordion with four sections:

### Posts Tab (📝)
- Create, edit, delete blog posts
- Each post can target a directory/network
- Published posts appear on directory landing pages and in RSS/sitemap feeds

### Q&A Automation Tab (❓)

Automatically generate SEO-optimized Q&A content from seed keywords.

**Fetch Keywords:** Enter seed keywords (comma-separated), select source (AI Generated, AnswerThePublic, or DataForSEO), and a Directory ID. The system generates 50+ common questions people search for related to those keywords using your configured AI provider.

**Keyword Pool:** Filter by Directory ID, Status (unused/drafted), and Source. Shows the accumulated keyword bank.

**Generate Posts:** Pick N unused keywords (max 20), specify the target Directory ID. The system writes full 800-word blog posts using AI, stores them as drafts, and marks the keywords as "drafted".

**Newsletter Digest:** Creates a weekly roundup HTML newsletter from the latest published Q&A posts. Option to send immediately or schedule weekly (configures day/time per directory).

### Trapdoors Tab (🔲)
For configuring trapdoor keywords and landing pages for SEO capture. (Feature in development — placeholder shown.)

### SEO Tab (🔍)
Quick links to:
- **Google News Sitemap** — auto-populated from last 48 hours of published posts. One per directory in the network. Zero admin touch.
- **RSS Feed** — last 50 published posts. One per directory.
- **Articles Sitemap** — all published articles via lastmod.
- **Related Articles** — auto-displayed at bottom of blog posts within the same directory.

### Search Source Dropdown

The Search page now includes a source dropdown (All, Web, Blog, Q&A, News) at the top. Results include source badges and route through the output engine. 

### Integration Center (🔑)

The Integrations page now has two sections:
1. **CTA Button Config** — per-business action buttons (Booking, Campaign, SMS) — existing behavior unchanged
2. **API Integrations** — configure API keys for:
   - **AnswerThePublic** — API key for keyword discovery
   - **DataForSEO** — Login + API key for competitor keyword data
   - **AI Provider** — API key + model selection (gpt-4o-mini, deepseek-chat, etc.) for content generation

API keys are stored encrypted in the `integration_configs` table and masked in the UI (showing only last 4 chars).

## Micro-Polls

**Location:** Admin Sidebar → Polls

Create one-question polls for your directory. Visitors vote and see live results.

### Creating a Poll
1. Click **Add Poll** from the Polls page
2. Enter the question (e.g. "What type of event should we host this month?")
3. Add options (click + to add rows, ✕ to remove)
4. Set an optional end date — polls auto-close
5. Choose the directory — poll appears as a sidebar widget on that directory's listing pages

### Management
- **Active** polls show with vote counts and percentage bars
- **Close Poll** button to end voting early
- **Closed** polls are read-only — results visible but no new votes
- Each visitor votes once per poll; they can change their vote anytime

### API Reference

| Endpoint | Auth | Purpose |
|---|---|---|
| `POST /api/v1/polls` | Admin JWT | Create poll |
| `GET /api/v1/polls?directory_id=X&status=active` | Public | List polls |
| `GET /api/v1/polls/:id` | Public | Get poll with results |
| `POST /api/v1/polls/:id/vote` | Visitor JWT | Cast/change vote |
| `POST /api/v1/polls/:id/close` | Admin JWT | Close poll |

Polls are visible to all visitors — no login required to see the question and results. Voting requires a free visitor account.

## Neighborhood Feed

**URL:** `zaarhub.com/feed` (visitor JWT required)

A personalized homepage for visitors that aggregates:
- Their bookmarked businesses
- Upcoming events they've RSVP'd to
- Active polls in their directory
- Business suggestions based on bookmarked categories + survey answers

The feed is fully automated — no admin setup needed. Visitors see it when logged in.

API: `GET /api/v1/feed` returns full JSON structure for custom frontends.

## Deal Management

Deals have visual templates (classic, modern, bold, minimal), countdown timers, customizable CTA buttons and colors, gallery images, and rotation scheduling. Deal pages at `zaarhub.com/deals/{id}`.

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
