# Directory Admin Guide — MultiDirectory Platform

**For:** SwiftSoftware Admin (David) — the directory builder operating this platform.

## Architecture

MultiDirectory is a **single-instance directory platform**, not a SaaS. Each directory (ZaarHub, Bob's Auto Shop, etc.) is a standalone product you build, configure, and sell. When a directory is sold, the purchaser either downloads their codebase or gets admin access while you host it.

### User Roles

| Role | Who | What They Can Do |
|---|---|---|
| **super_admin** | David (SwiftSoftware owner) | Full system access, all directories |
| **directory_admin** | Directory owners/operators | Manage their directory's listings, deals, config |
| **business** | Business owners on a directory | Manage their listing, deals, messages |
| **supplier** | Wholesalers, farms, distributors | Manage product catalog, B2B orders |
| **visitor/customer** | End users browsing directories | Search, bookmark, claim deals, RSVP |

### What "Tenant" Never Means

This is NOT a SaaS. There are no tenants paying rent. A directory IS a standalone product. Within each directory, you have:
- **Businesses** — companies listed on the directory
- **Suppliers** — B2B vendors providing goods/services to businesses
- **Customers/Visitors** — people browsing and interacting with the directory

## Managing Directories

From the admin dashboard, manage any directory you operate. These are the key sections:

## Listings
Add, edit, or remove business listings. Set business type (local, supplier, distributor, farm, etc.). Manage claim status.

## Deals
Create deals with templates — percentage off, fixed price, BOGO, free gift. Set colors, CTA text, timer, gallery images. Track redemptions by code. Businesses can feature a deal on their listing with a custom CTA button.

## Monetization
Create plan tiers (Listed/Featured/Premium). Set monthly/yearly pricing. Grandfather early subscribers. **Tier limits:** Free = 0 deals, Featured = 3 active deals, Premium = 10 active + 5 scheduled rotations.

## Categories
Organize listings with categories.

## Blog & SEO Articles
Write blog posts. Generate AI-optimized articles for businesses. Community posts appear on the city community page.

## Data Import
Add your API keys in Settings → API Keys, then run imports from the Data Import panel. Choose Businesses or Suppliers — results go to the right directory automatically.

## City Requests
Visitors can request cities that aren't listed yet. Manage them under **City Requests** in the sidebar:
- View all requested cities ranked by vote count
- Green badge = already added, gray = pending
- Click "Mark Added" when you launch a requested city
- This helps prioritize which cities to add next

## Visitor Bookmarks
Visitors can save businesses they like. Bookmark counts appear on each business listing — useful for gauging popularity.

## Community Polls
Create one-question polls from the admin panel → **Polls** in sidebar:
- Question + up to 8 options + optional end date
- Set directory, status management (active/closed), live results with percentage bars
- Visitors vote once per poll, see live results after voting
- Active polls show as a sidebar widget on directory pages

## Branding
Custom colors, logos, favicon per directory.

## Legal Pages — Per-Directory Configuration

Each directory gets its own set of legal pages, fully editable from the admin panel at `/directory-admin.html`.

### Managing Legal Pages
- Go to **Settings → Legal Pages** in the directory admin panel
- Create or edit pages: Terms of Service, Privacy Policy, Contact, Disclaimer, Loyalty Terms, or any custom page
- Each page has: slug (URL path), title, HTML content, publish status, footer visibility toggle
- Set **Show in Footer** to include a legal page in the site-wide footer
- Reorder pages with display_order

### Branding Configuration
- **Site Name** — the name shown in headers, footers, and page titles
- **Tagline** — directory slogan shown on city landing pages
- **Primary/Secondary Colors** — brand colors used across the directory
- **Logo URL** — header logo image
- **Favicon URL** — browser tab icon
- **Copyright Year** — shown in the footer (e.g., "2026" or "2024–2026")
- **Contact Email/Phone** — shown on contact page and footer
- **Analytics & Social** — Google Analytics ID, Facebook App ID, Twitter handle

### Cookie Consent Banner
- Appears at the bottom of every page until the visitor accepts
- "Accept All" enables full analytics tracking
- "Essential Only" enables only functional cookies
- Preference stored in browser localStorage
- Links to Privacy Policy and Terms of Service in the banner

### Legal Page URLs
Each directory's legal pages are served at:
- `/legal/terms`
- `/legal/privacy`
- `/legal/contact`
- `/legal/disclaimer`
- `/legal/loyalty`
- `/legal/{any-custom-slug}`

All legal pages are SSR-rendered with full SEO tags, branded headers, and the directory's footer.

### City SSR Pages
Each directory gets SEO-optimized city landing pages:
- `/zaarhub` — all cities index
- `/zaarhub/:slug` — individual city page with business listings, ratings, categories
- `/zaarhub/:slug/:business_id` — business detail page with schema.org LocalBusiness JSON-LD
- All pages include: meta tags, Open Graph tags, canonical URLs, star ratings, offer/claim flow

## Service Catalog Management

Manage directory-level services and service locations. These appear on service-location pages that help your directory rank for "service in city" searches.

### Managing Services
- Go to **Services** in the admin sidebar
- Add a new service with name, description, category, and icon
- Edit or remove existing services — changes apply directory-wide
- CSV import: upload a spreadsheet of services and locations in bulk

### Service-Location Pages
Each service + location combination can generate a dedicated landing page. For example, a "Plumbing" + "Palm Bay" page ranks for "plumbers in Palm Bay" in search engines.

- **Create locations:** Add cities, neighborhoods, or service areas under **Locations** in the sidebar
- **Link services to locations:** From the service editor, associate a service with specific locations
- Each link may generate a public page at `/{directory}/{location}/{service}`

### Endpoints
```
GET    /api/v1/services                              — List all services
POST   /api/v1/services                              — Create service
GET    /api/v1/services/:id                          — Get service details
PUT    /api/v1/services/:id                          — Update service
DELETE /api/v1/services/:id                          — Delete service
GET    /api/v1/directories/:id/services              — List directory services
POST   /api/v1/directories/:id/services              — Create directory-level service
PUT    /api/v1/directories/:id/services/:svc_id      — Update directory service
DELETE /api/v1/directories/:id/services/:svc_id      — Delete directory service
GET    /api/v1/directories/:id/locations             — List locations
POST   /api/v1/directories/:id/locations             — Create location
PUT    /api/v1/directories/:id/locations/:loc_id      — Update location
DELETE /api/v1/directories/:id/locations/:loc_id      — Delete location
POST   /api/v1/directories/:id/services/import       — CSV import services
POST   /api/v1/directories/:id/locations/import       — CSV import locations
```

## Events — Moderation & Management

All events created by businesses go through the events system. You can moderate, feature, and manage all events across your directories.

### Moderation
- View all events in **Events** → **All Events**
- Filter by status: active, cancelled, completed
- Edit any event's details — correct titles, fix dates, update locations
- Cancel events that violate guidelines — a cancellation notice appears on the event page

### Featuring Events
- Toggle **Featured** on any event to pin it to the top of the city's events page
- Featured events also appear on the ZaarHub homepage
- Use featured events to highlight community gatherings, festivals, or sponsored events

### Attendee Management
- Click any event to see the attendee list
- View who RSVP'd, their RSVP status (going/maybe), and contact info
- Export attendee lists for event organizers

### Endpoints
```
GET    /api/v1/events                          — List all events (admin filters)
GET    /api/v1/events/:id                      — Event detail
POST   /api/v1/events/:id/edit                 — Edit any event (admin override)
POST   /api/v1/events/:id/cancel               — Cancel any event
GET    /api/v1/events/:id/attendees            — View attendee list
```

## CTA Configuration

CTAs (Call-to-Action buttons) appear on every business and supplier listing. You control the CTA system across all directories.

### The 13 CTA Types

| CTA Type | Best For |
|----------|----------|
| **Get a Quote** | Service businesses (contractors, plumbers, landscapers) |
| **Book Now** | Appointment-based businesses (salons, doctors, consultants) |
| **Call Now** | Businesses that want phone leads |
| **Visit Website** | Businesses with a strong website presence |
| **Message Us** | Any business — opens the messaging form |
| **Join Rewards** | Businesses using IncentiveSwift loyalty programs |
| **Claim Deal** | Businesses running active deals |
| **View Menu** | Restaurants and food services |
| **Get Directions** | Physical locations (retail, restaurants, offices) |
| **Email Us** | Businesses that prefer email inquiries |
| **Download App** | Businesses with a mobile app |
| **Donate** | Nonprofits and charitable organizations |
| **Register** | Classes, workshops, and event-based businesses |

### Gated CTAs

You can require visitors to log in before using certain CTAs. This is configured per directory:

- **Enable gated CTAs** in Directory Settings → CTA Configuration
- Choose which CTA types require login (commonly "Book Now" and "Get a Quote")
- Visitors see a login/register prompt when clicking a gated CTA
- After authenticating, they're redirected to complete the action

### Where CTAs Are Stored

Each business's CTA type is stored in its meta fields at `POST /api/v1/businesses/:id/meta` with the key `cta_type`. Suppliers use the same system at `POST /api/v1/suppliers/:id/meta`.

### Best Practices
- Set sensible defaults per category (restaurants default to "View Menu", contractors to "Get a Quote")
- Monitor which CTA types generate the most engagement
- Standardize CTAs across similar businesses for a consistent visitor experience

## Connected Services Management

Configure directory-level connections between directories and SwiftSoftware products (IncentiveSwift and CoreSwift CRM) on behalf of business owners.

### How It Works

Business owners connect their IncentiveSwift and CoreSwift accounts from their portal. As admin, you can:

- **View connected services** per business — see who's connected to what
- **Troubleshoot connections** — check verification status and API key validity
- **Disconnect services** — if a business owner leaves or needs support
- **Configure connection defaults** — set which services are available per directory

### Per-Directory Configuration

- Go to **Directory Settings → Connected Services**
- Toggle which services are available for this directory:
  - **IncentiveSwift** — enable/disable campaign linking
  - **CoreSwift CRM** — enable/disable booking and inbox sync
- Set default connection templates for new businesses

### Endpoints
```
GET    /api/v1/connected-services                     — List all connections
POST   /api/v1/connected-services/connect             — Connect a service (on behalf of business)
POST   /api/v1/connected-services/verify              — Verify API key validity
DELETE /api/v1/connected-services/disconnect           — Disconnect a service
```

## Unified Inbox Routing

When a business owner connects CoreSwift CRM, directory messages are forwarded to their CRM inbox. As admin, you manage the routing rules.

### Auto-Routing Rules

Create rules that determine how messages flow between the directory and CoreSwift CRM:

- **Default route:** Messages go to the business's CoreSwift inbox automatically when connected
- **Fallback routing:** If CoreSwift connection is down or expired, messages stay in the directory inbox
- **Category-based routing:** Route messages differently based on business category or type
- **Priority routing:** Flag certain messages (e.g., from VIP visitors or quote requests) for priority handling

### Webhook Configuration

The directory sends messages to CoreSwift at: `POST /api/messages/webhook` (CoreSwift port 8084)

Message CRUD at CoreSwift: `GET/POST/PUT/DELETE /api/messages`

### Monitoring

- Check webhook delivery logs for failed forwards
- Retry failed deliveries manually from the admin panel
- View sync status per business — see if their inbox is in sync

## Sponsors & Ads Management

Run an advertising system on your directory. Businesses pay for sponsored placements — you set the pricing, approve creatives, and manage ad inventory.

### Sponsor Approval Workflow

1. Business owner purchases a sponsorship from their portal
2. They upload creative — an image, headline, description, and destination URL
3. The sponsorship appears in your **Sponsors → Pending Approval** queue
4. You review the creative:
   - Check the image meets quality and size guidelines
   - Verify the headline and description are appropriate
   - Confirm the destination URL works and is safe
5. **Approve** — the ad goes live in its scheduled slot
6. **Reject** — send a reason back to the business owner so they can fix and resubmit

### Setting Pricing

- Go to **Monetization → Sponsor Pricing**
- Set prices for each ad slot:
  - **Homepage banner:** premium placement, highest price
  - **Sidebar ad:** good visibility, moderate price
  - **Featured listing:** appears at top of searches, per-category option
  - **Category sponsor:** exclusive sponsor for a category
- Choose pricing model: monthly flat fee, per-impression, or per-click

### Managing Ad Slots

- View all active and scheduled ads in **Sponsors → Ad Slots**
- See which slots are filled, available, or expiring soon
- Reassign slots if a sponsorship is cancelled
- Set rotation schedules — multiple ads can rotate through the same slot

### Sponsor Stats

- Track impressions, clicks, and CTR per sponsor
- Revenue dashboard: total ad revenue by directory, month, and slot type
- Export sponsor reports for billing

### Endpoints
```
GET    /api/v1/sponsors                              — List sponsors
POST   /api/v1/sponsors                              — Create sponsor record
GET    /api/v1/creatives                             — List ad creatives
POST   /api/v1/creatives                             — Upload creative
GET    /api/v1/sponsor-schedules                     — View ad schedules
POST   /api/v1/sponsor-approvals                     — Approve/reject sponsors
```

## Landing Pages — Theme Library & Settings

Business owners can create custom landing pages. As admin, you manage the theme library and per-directory landing page settings.

### Managing Themes

- Go to **Landing Pages → Themes** to see your theme library
- Add new themes — HTML/CSS templates that business owners can choose from
- Edit existing themes — update styles, layouts, and default content blocks
- Feature or retire themes — highlight new themes, hide outdated ones

### Per-Directory Settings

- Turn landing pages on or off per directory
- Set which themes are available per directory
- Configure custom domain support for landing pages (e.g., `businessname.com`)
- Manage published pages — view all live pages, unpublish or republish

### Endpoints
```
GET    /api/v1/landing-pages                         — List landing pages (admin)
POST   /api/v1/landing-pages                         — Create landing page
PUT    /api/v1/landing-pages/:id                     — Update landing page
DELETE /api/v1/landing-pages/:id                     — Delete landing page
POST   /api/v1/landing-pages/publish                 — Publish a landing page
GET    /api/v1/public-pages                          — List all public pages
```

## SEO Configuration

Optimize how your directories appear in search engines. Good SEO means more visitors find your directory through Google, Bing, and other search engines.

### Meta Tags

Go to **SEO → Meta Tags** per directory:
- **Title template:** Default format for page titles (e.g., `{business_name} — {directory_name}`)
- **Meta description template:** Default description for pages (e.g., "Find the best {category} in {city}. Browse reviews, deals, and more.")
- **Open Graph tags:** Control how your pages look when shared on Facebook, Twitter, and LinkedIn
- **Custom meta per page:** Override defaults for specific pages (homepage, category pages, etc.)

### Schema.org Structured Data

The system automatically generates structured data (JSON-LD) for:
- **LocalBusiness schema** on every business listing page — includes name, address, phone, rating, reviews
- **BreadcrumbList schema** on category and subcategory pages
- **Event schema** on event detail pages
- **FAQ schema** on blog posts using the FAQ template

Configure schema settings in **SEO → Structured Data** — toggle which schemas are enabled per directory.

### Sitemaps

- Auto-generated XML sitemaps at `/{directory}/sitemap.xml`
- Includes all businesses, categories, events, blog posts, and landing pages
- Updates automatically when content changes
- Submit your sitemap URL to Google Search Console for faster indexing

### Canonical URLs

- Every page gets a canonical URL to prevent duplicate content penalties
- Custom domains use their own canonical URLs
- Pagination pages use proper `rel="next"` and `rel="prev"` tags

### Endpoints
```
GET    /api/v1/seo                                  — Get SEO settings
PUT    /api/v1/seo                                  — Update SEO settings
GET    /api/v1/seo/directory/:id                    — Get directory-specific SEO
PUT    /api/v1/seo/directory/:id                    — Update directory SEO
GET    /api/v1/seo/sitemap                          — Generate/download sitemap
```

## Tag Automation

Auto-tag visitors based on their behavior. Tags help you and business owners target the right audience for emails, deals, and campaigns.

### How Auto-Tag Rules Work

1. You create a rule: "If a visitor does X, tag them Y"
2. The system watches visitor behavior in real time
3. When a visitor matches a rule, the tag is automatically applied to their profile
4. Business owners can then target their email campaigns by tag

### Creating Auto-Tag Rules

Go to **Automation → Tag Rules** and click **Create Rule**:

- **Trigger:** What action triggers the tag
  - Visitor bookmarks a business in a specific category
  - Visitor claims a deal
  - Visitor RSVPs to an event
  - Visitor searches for a specific keyword
  - Visitor visits a certain number of pages
- **Condition:** Optional filters (only apply if the visitor is in a certain city, etc.)
- **Tag to apply:** The tag name (e.g., "foodie", "homeowner", "event-goer", "deal-hunter")

### Example Rules

| Rule | When | Tag |
|------|------|-----|
| Bookmarked 3+ restaurants | Visitor saves 3 restaurants | "foodie" |
| Claimed a deal | Visitor claims any deal | "deal-hunter" |
| RSVP'd to event | Visitor RSVPs to any event | "event-goer" |
| Searched "emergency" | Visitor searches for emergency services | "urgent-need" |
| Viewed 5+ plumbers | Visitor browses plumbing businesses | "homeowner" |

### Using Tags

- Tags appear on visitor profiles in the admin panel
- Business owners see tag counts when composing email campaigns
- Export tagged visitor lists for remarketing

### Endpoints
```
GET    /api/v1/auto-tag-rules                       — List all auto-tag rules
POST   /api/v1/auto-tag-rules                       — Create auto-tag rule
GET    /api/v1/auto-tag-rules/:id                   — Get rule details
PUT    /api/v1/auto-tag-rules/:id                   — Update rule
DELETE /api/v1/auto-tag-rules/:id                   — Delete rule
```

## Tracked Links

Create short, trackable links to measure clicks and engagement. Perfect for sharing on social media, in emails, or in ads.

### Creating a Tracked Link

1. Go to **Marketing → Tracked Links**
2. Click **Create Link**
3. Fill in:
   - **Destination URL:** Where the link should go
   - **Short code:** Custom short link (e.g., `zaarhub.com/go/summer-sale`)
   - **Campaign name:** Optional label to group related links
   - **Source:** Where you'll share this link (social, email, ad, etc.)
4. Click **Create** — your short link is ready to use

### Viewing Stats

Click any tracked link to see:
- **Total clicks** — how many times the link was clicked
- **Clicks by date** — daily breakdown chart
- **Clicks by source** — which platforms drove the most traffic
- **Clicks by location** — where clickers are located (city/country)
- **Unique clicks** — how many unique visitors clicked (vs repeat clicks)

### Use Cases

- Share a tracked link to your directory on social media — see which platform drives the most traffic
- Give business owners tracked links to their listings — they can measure their own marketing
- Track email campaign performance — use different links for different emails

### Endpoints
```
GET    /api/v1/tracked-links                        — List all tracked links
POST   /api/v1/tracked-links                        — Create tracked link
GET    /api/v1/tracked-links/:id/stats              — Get click statistics
PUT    /api/v1/tracked-links/:id                    — Update link
DELETE /api/v1/tracked-links/:id                    — Delete link
```

## Email Campaigns — Admin Management

Oversee email campaigns across all directories. Manage templates, review campaigns before they send, and track performance.

### Campaign Management

- Go to **Email → Campaigns** to see all scheduled, sending, and completed campaigns
- Filter by directory, business, or status
- Review campaign content before it sends — approve or reject
- Cancel campaigns that violate your directory's email policy

### Template Management

- Create reusable email templates in **Email → Templates**
- Templates can include merge fields: `{{business_name}}`, `{{visitor_name}}`, `{{directory_name}}`, `{{deal_title}}`
- Business owners choose from approved templates when creating campaigns
- Update templates — all future campaigns using that template get the update

### Compliance

- All emails include auto-generated unsubscribe links
- The system tracks unsubscribe requests per directory
- Set sending limits per business to prevent spam
- Monitor bounce rates and flag problematic senders

### Endpoints
```
GET    /api/v1/email-templates                      — List email templates
POST   /api/v1/email-templates                      — Create template
PUT    /api/v1/email-templates/:id                  — Update template
DELETE /api/v1/email-templates/:id                  — Delete template
GET    /api/v1/email-campaigns                      — List all campaigns
POST   /api/v1/email-campaigns                      — Create campaign
```

## Export System

Export any data from your directory in CSV or JSON format. Use export templates to save common export configurations.

### What You Can Export

| Data Type | Format | What's Included |
|-----------|--------|-----------------|
| **Businesses** | CSV, JSON | Name, category, address, phone, email, website, rating, review count, plan tier |
| **Reviews** | CSV, JSON | Business name, reviewer, rating, title, review text, date, status |
| **Visitors** | CSV, JSON | Name, email, phone, tags, bookmark count, deal claims, last activity |
| **Deals** | CSV, JSON | Business, deal title, type, claims, redemption count, dates |
| **Events** | CSV, JSON | Title, business, date, location, RSVP count, status |
| **Messages** | CSV, JSON | Sender, recipient business, subject, date, read status |
| **Sponsors** | CSV, JSON | Business, slot type, impressions, clicks, spend, dates |
| **Call Logs** | CSV, JSON | Tracking number, caller, duration, date, status |

### Export Templates

Save time with templates — pre-configured exports you can run with one click:

- **Create a template:** Select data types, filters, columns, and format → save as a named template
- **Run a template:** One click exports the data with your saved settings
- **Schedule exports:** Set a template to run daily, weekly, or monthly — results emailed to you

### Endpoints
```
GET    /api/v1/exports                              — List past exports
POST   /api/v1/exports                              — Create a new export
GET    /api/v1/exports/:id                           — Download export file
GET    /api/v1/exports/templates                    — List saved templates
POST   /api/v1/exports/templates                    — Create export template
PUT    /api/v1/exports/templates/:id                — Update template
DELETE /api/v1/exports/templates/:id                — Delete template
```

## Data Import — CSV, Yelp & Places

Import business data from multiple sources to quickly populate your directories.

### CSV Import

1. Go to **Data Import → CSV**
2. Download the template CSV (has the correct column headers)
3. Fill in your business data — name, category, address, phone, email, website, description
4. Select the target directory and business type (business or supplier)
5. Upload the CSV file
6. Review the preview — the system shows you which rows will be imported
7. Confirm — businesses are created in your directory

### Yelp Enrichment

Enrich existing business listings with data from Yelp:

- Connect your Yelp Fusion API key in **Settings → API Keys**
- Go to **Data Import → Yelp Enrichment**
- Select businesses to enrich — the system fetches Yelp ratings, review snippets, photos, and hours
- Review enriched data before applying — pick and choose what to import

### Places Autocomplete

Speed up manual business entry with Google Places autocomplete:

- Connect your Google Places API key in **Settings → API Keys**
- When adding a business, start typing the name — matching Google Places suggestions appear
- Select a suggestion — name, address, phone, website, and coordinates auto-populate
- Review and edit any field before saving

### Endpoints
```
GET    /api/v1/imports                              — List import history
POST   /api/v1/imports                              — Start a new import
POST   /api/v1/imports/csv                          — Upload CSV for import
GET    /api/v1/imports/:id                           — Check import status
```

## Call Tracking — Phone Number Pool

Manage the pool of tracking phone numbers used by businesses in your directories.

### Provisioning Numbers

- Go to **Settings → Call Tracking**
- Purchase or provision phone numbers from your telephony provider
- Each number is added to the pool and marked as **available**
- Set up call forwarding — calls to tracking numbers are routed to the business's real number

### Assigning Numbers

- When a business owner requests a tracking number, assign one from the pool
- The number status changes from **available** to **assigned**
- Multiple businesses cannot share the same tracking number — one number per business
- Release a number back to the pool when a business cancels or churns

### Call Logs

- View all call logs across all directories in **Call Tracking → Logs**
- Filter by directory, business, date range, or call status
- See: tracking number, business, caller number, start time, duration, recording URL (if available)
- Export call logs as CSV for reporting or billing

### Call Stats

The admin call stats dashboard shows:
- **Total calls per directory** — compare directories
- **Average call duration** — overall and per directory
- **Most-called businesses** — which listings generate the most phone leads
- **Call trends** — weekly and monthly call volume charts

### Endpoints
```
GET    /api/v1/phone-numbers                        — List all phone numbers in the pool
POST   /api/v1/phone-numbers                        — Add a phone number to the pool
PUT    /api/v1/phone-numbers/:id                    — Update phone number (assign/release)
DELETE /api/v1/phone-numbers/:id                    — Remove from pool
GET    /api/v1/call-logs                            — List call logs
GET    /api/v1/call-stats                           — Get aggregate call statistics
```
