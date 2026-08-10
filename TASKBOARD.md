# Task Board — Builder Agent

## Active Tasks

### Loyalty Display — Business Profiles
**Added:** 2026-08-10
**Context:** Add loyalty section to each business profile page. Show program name, member tier, points balance, QR code. Scan to Earn button links to /scanner.
**APIs:** GET /api/v1/loyalty/programs, GET /api/v1/loyalty/member/{id}, GET /api/v1/loyalty/member/{id}/qr
**Status:** In Progress

## Backlog

### Content Research Engine — Admin UI
**Added:** 2026-07-31
**Context:** Backend is built (topics x cities x sources cross-product, AI drafting). Need admin UI to configure sources, trigger research runs, view drafts.
**Status:** Backend done, UI needed

### Blog Features — Content Decay Dashboard
**Added:** 2026-08-02
**Context:** Content decay tracking, internal linking, AEO scoring all built in backend. Need visual dashboard showing decay timeline, link health, AEO scores per post.
**Status:** Backend done, UI partial

### AEO Scoring — Frontend Display
**Added:** 2026-08-02
**Context:** Answer-first structure scoring (criterion #9) implemented. Need score display in blog editor — show AEO score breakdown per post before publishing.
**Status:** Backend done, UI needed

### Scanner PWA — Real-World Testing
**Added:** 2026-08-10
**Context:** Scanner deployed at zaarhub.com/scanner. Test on real phones with actual QR codes. Improve camera auto-focus, add scan history log.
**Status:** Needs phone testing

### Category System — Admin UX Polish
**Added:** 2026-07-29
**Context:** Multi-category system (search filter, bulk assign, requests) working. Admin UX needs drag-and-drop reordering, batch operations, category merge tool.
**Status:** Working, needs polish

### Business Booking System Integration
**Added:** 2026-08-10
**Context:** Wire booking system to business profiles. Calendar integration, confirmation emails via n8n.
**Status:** Planned

### B2B Marketplace — RFQ Workflow
**Added:** 2026-07-30
**Context:** RFQ marketplace backend built. Need frontend for businesses to post RFQs, suppliers to bid, and matching algorithm UI.
**Status:** Backend done, UI needed

### Lead Sharing Network — Admin Dashboard
**Added:** 2026-07-30
**Context:** Lead sharing network backend built. Need admin dashboard for lead routing, acceptance tracking, and commission reporting.
**Status:** Backend done, UI needed

## In Review

—

## Completed

### Scanner PWA (Aug 10, 2026)
- zaarhub.com/scanner with jsQR camera scanning
- Three scan types: Check-in, Purchase, Redeem
- PWA manifest + service worker
- Proxies to IncentiveSwift scan endpoint

### Agent Rules — Self-Verification (Aug 9, 2026)
- Read-only protection header directive
- Admin login verification — never break admin auth
- Customized agent rules for multi-directory

### Blog Features Handler (Aug 2, 2026)
- Content decay tracking + internal linking + AEO scoring
- Schema markup generation
- Admin UI page (4 tabs)
- Bug fixes: directories->tenants table refs, sqlx::query!->query_as

### Content Research Engine (Jul 31, 2026)
- Q&A topic research with trending data
- Bulk research: topics x cities x sources cross-product
- AI drafting integration
- Traffic trend into decay priority

### Category Restructure (Jul 29, 2026)
- Dining group with Fine Dining & Cuisine, Food & Drink
- Multi-category policy for businesses
- 10 new categories added

### B2B Deep Moat (Jul 30, 2026)
- RFQ marketplace backend
- Lead sharing network
- Co-op buying groups

### Competitive Moat Quick Wins (Jul 30, 2026)
- Verification badges system
- Data freshness indicators
- Editor's picks

### Platform Features (Jul 29, 2026)
- Ported ZaarHub SSR, legal pages, analytics from FunnelSwift
- QR codes, template variants, social share (premium gated)
- Config panel additions, premium upsell, per-user limits
- Comprehensive guide documentation — all 15 platform features

### ZaarHub Branding Fix (Jul 29, 2026)
- Restored original ZaarHub logo — globe emoji + Community suffix
