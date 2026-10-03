# ZaarHub / Multi-Directory — TEMPLATE vs CONFIGURABLE vs CUSTOM ledger

**Card:** B94 · **Status:** living document — initial classification shipped with the first
commit; every subsequent card MUST classify each item of work as it lands.
**Purpose (David, 2026-10-01):** *"make notice so when we're finally done you can itemise the
things that would be considered CUSTOMIZED FOR ZAARHUB."* When a directory is SOLD, David must
be able to say exactly what a buyer receives, and separately what was custom to ZaarHub (and is
therefore either excluded or must be productised before sale).

## The classification

| Tag | Meaning |
|-----|---------|
| **[TEMPLATE]** | The **mechanism** itself — ships in every directory out of the box. |
| **[CONFIGURABLE]** | Templated, but the **value** is editable in the affiliated directory's / network's own admin panel — a buyer changes it without code and without David. This is the default home for anything with a value. |
| **[CUSTOM]** | Built for ZaarHub specifically; a buyer would not get it unless it is promoted to template. Rare by design; each instance is a liability when selling. |

**Rule of thumb: MECHANISMS ARE TEMPLATE; VALUES ARE CUSTOM.** What makes ZaarHub ZaarHub is
its own data, copy and choices — not special code.

## Platform mechanisms — [TEMPLATE]

Every one of these is code that ships in any directory instance and is admin-editable per
directory/network:

* **Directory & network engine** — `directories` (per city), `networks` (a network groups cities
  or is a standalone directory), host→directory resolution, network→directory→default brand
  inheritance.
* **Public SSR surfaces** — subfolder city pages (`/<city>/…`), city landing, businesses list &
  detail, articles, blog list/post, deals, sitemap.xml, robots.txt, programmatic pages.
* **Loyalty engine (native)** — `loyalty_programs`, members/enrollments, check-ins, scans,
  tiers, rewards, milestones, redemption, point ledger. Currency name/icon/colour and every
  rate are per-programme values (earn rate, redemption cap %, minimum balance, per-check-in).
* **Discovery & enrichment** — business search, review queue, field-level merge contract,
  sources (Google Places, OpenStreetMap/Overpass, business website, registries).
* **Suppliers / B2B** — supplier directory, products, orders, messages, RFQ marketplace, co-op
  hub, lead exchange.
* **Admin console** — `admin-panel.html` with sections: directories & networks, provider keys,
  payment gateways, ad zones & ad revenue, data enrichment, demand analytics, domains, email
  settings, industries, loyalty programmes, sitemap/SEO config, content research (sc-* sections),
  submissions, subscribers, transfers, legal pages, system email templates.
* **Auth & identity** — visitor/customer accounts, business-owner claim + dashboard, supplier
  portal, admin/operator roles, API keys.
* **Commerce plumbing** — plans & subscriptions, checkout handlers, clearinghouse / settlement
  runs, monetization (ad earnings), payment webhooks.
* **Onboarding & CRM seam** — native onboarding questionnaire builder, answer→data-point mapping,
  CoreSwift native integration (connect / test / status / disconnect; per-directory and
  per-network credentials stored encrypted in the DB).
* **Content systems** — blog generator/QA/SEO, articles feed, content queue, email templates,
  newsletter, announcements, audience segments.
* **Observability & automation** — analytics, visitor tracking/beacon, webhooks, integrations,
  export/import (CSV) per entity, probe-harness row marking.

## Per-directory / per-network values — [CONFIGURABLE]

These are the **values** a buyer (or a VA) sets in the panel; none are hardcoded:

* Branding: site name, logo, colours/theme, custom domain, SEO overrides per page.
* Loyalty programme: currency name/icon/colour, earn rate, redemption cap %, minimum balance,
  per-check-in/per-visit rates, tiers, rewards, milestones, exclusions.
* Provider keys: Google Maps (server) & Google Places, and every other named source — labelled,
  encrypted at rest, masked preview, set/change/deactivate.
* Payment provider + credentials; clearinghouse settlement settings; ad zones and plans/tiers.
* Email/SMTP transport + sending identity per directory, and system email templates.
* Legal pages (slug + body) per directory/network, auto-linked into every footer.
* Onboarding questionnaires, field mappings, CoreSwift account link + per-audience list ids.
* Categories/industries, navigation, homepage composition, sitemap config.

## ZaarHub-specific — [CUSTOM] (must be itemised before any sale)

Anything below is a **value or choice**, not special code — it stays [CUSTOM] in the sense that a
buyer would receive the mechanism with empty defaults unless the data is exported or reproduced:

* The **ZaarHub brand** (name, logo, colours, copy) and the **10-city network dataset**
  (Palm Bay + 9 cities created 2026-07-18) with ~4,000 businesses.
* The **Palm Bay = first-created = "main admin"** convention. **FLAG:** this is implicit
  (creation order), not a real setting — a fragility to fix (see B97).
* ZaarHub vertical content: hotel-savings FAQ/terms, restaurant-certificates FAQ,
  vacation-incentive terms, the specific legal-page bodies authored for ZaarHub.
* ZaarHub-specific loyalty configuration (network-wide programme, rate values).
* Legacy artefacts retained for ZaarHub only: `incentiveswift-terms.html`, retired
  IncentiveSwift connect flow references.

> Nothing here is custom *code*. If a future card adds genuinely custom code, it MUST be flagged
> in this table and scheduled for promotion to template (or documented as excluded from sale).

## Maintenance rule

Every card that lands classifies its work above. A card that adds a mechanism → [TEMPLATE];
a value an admin can edit → [CONFIGURABLE]; a ZaarHub-only build/choice → [CUSTOM]. Re-check this
file when preparing a sale or a handover (B85).
