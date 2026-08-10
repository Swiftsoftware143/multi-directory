# Task Board — Builder Agent

## Active Tasks

### Scanner PWA — Phone Testing
**Added:** 2026-08-10
**Assigned by:** Lead Architect (19:41 ET)
**Status:** Active — needs real-world testing
**Context:** Test zaarhub.com/scanner on a real phone with a ZaarHub loyalty QR code:
- Camera opens and detects QR within 2 seconds
- Three scan type buttons all work (Check-in, Purchase, Redeem)
- Purchase scan shows points awarded
- Success overlay shows member name, tier badge, points
- Test on both iOS Safari AND Android Chrome
- Report results to Lead Architect

### Loyalty Display — Business Profiles
**Added:** 2026-08-10
**Status:** Deployed — verify on phone
**Context:** Loyalty section on zaarhub.com/biz/:id shows program, tier, balance, QR. Verify on phone browser.

## Backlog

### Content Research Engine — Admin UI
**Added:** 2026-07-31
**Status:** Backend done, UI needed

### Blog Features — Content Decay Dashboard
**Added:** 2026-08-02
**Status:** Backend done, UI partial

### AEO Scoring — Frontend Display
**Added:** 2026-08-02
**Status:** Backend done, UI needed

### Category System — Admin UX Polish
**Added:** 2026-07-29
**Status:** Working, needs polish

### Business Booking System Integration
**Added:** 2026-08-10
**Status:** Planned

### B2B Marketplace — RFQ Workflow UI
**Added:** 2026-07-30
**Status:** Backend done, UI needed

### Lead Sharing Network — Admin Dashboard
**Added:** 2026-07-30
**Status:** Backend done, UI needed

### Per-Directory Scanner Toggle
**Added:** 2026-08-10
**Context:** feature_config JSONB field exists — need admin UI to toggle scanner per directory deployment.
**Status:** Planned

### Member Profile Page — Full History
**Added:** 2026-08-10
**Context:** Member profile page with full scan history, points earned timeline, redemptions, tier progression.
**Status:** Planned

## Completed

### Scanner PWA (Aug 10, 2026)
- zaarhub.com/scanner with jsQR camera scanning
- Three scan types: Check-in, Purchase, Redeem
- PWA manifest + service worker

### Loyalty Display on Business Profiles (Aug 10, 2026)
- Loyalty card on zaarhub.com/biz/:id
- Fetches programs and tiers from IncentiveSwift
- Shows tier, balance, QR for enrolled members
- Join button for non-enrolled members

### Agent Rules — Self-Verification (Aug 9, 2026)
### Blog Features Handler (Aug 2, 2026)
### Content Research Engine (Jul 31, 2026)
### Category Restructure (Jul 29, 2026)
### B2B Deep Moat (Jul 30, 2026)
### Competitive Moat Quick Wins (Jul 30, 2026)
### Platform Features + Guides (Jul 29, 2026)
