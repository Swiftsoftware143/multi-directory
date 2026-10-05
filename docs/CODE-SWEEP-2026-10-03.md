# Multi-Directory code sweep — B101 (2026-10-03, lane=builds)

Card: *Code sweep: remove dead/legacy code across multi-directory*
Scope: `multi-directory` only. Build stayed green throughout (fmt/check/clippy + `gate-pre-build.sh`).

## 1. `points_per_redemption` — RESOLVED: live configuration, not dead code

`points_per_redemption` is a **real, routed, admin-editable loyalty setting**; it was
**not** removed (removing it would disable a capability).

* Schema: `loyalty_programs.points_per_redemption integer NOT NULL DEFAULT 0`
  (`000_baseline_live_schema.sql:1767`; COMMENT at :1774: *"credited to a member when they
  redeem a deal… 0 = disabled"*; originally added by legacy `094_points_per_redemption.sql`).
* Enforcement (the earn path): `src/handlers/deals.rs:634-677` reads the column and credits
  `rate_credit + points_per_redemption.max(0)` units on a deal redemption.
* Admin surface: `loyalty_native.rs` — CRUD (`ProgramInput`, create `INSERT`, update `SET`)
  and `frontend/admin-panel.html:4997` (`data-f="points_per_redemption"`, "…per redemption").
* Copy/state read: `loyalty_messaging.rs:229` (served "honest earn state").

**Meaning:** units of the directory currency credited when a member **redeems** a deal.
`0` = disabled. It is the *redemption-side* rate; `earn_rate` is the *spend-side* rate.
Documented here so it is never blind-deleted.

## 2. Removed (every removal proven unreferenced)

| # | What | Proof it was dead | Replacement / reason |
|---|------|-------------------|----------------------|
| R1 | `admin::admin_list_directories` (`src/handlers/admin.rs`) | not in `routes.rs`; no caller anywhere in `src/` | duplicate of `directories::list_directories`, routed at `routes.rs:27` |
| R2 | `networks::get_network_homepage`, `get_directory_homepage`, `create/update/get/delete_homepage_section` + `HomepageSection`, `CreateHomepageSectionRequest`, `UpdateHomepageSectionRequest` (`src/handlers/networks.rs`, `src/models/networks.rs`) | zero routes; zero callers; no frontend caller (`grep` over `frontend/ templates/` clean) | superseded by B86 `homepage_config` (`GET /homepage/config`, `GET/PUT /homepage-config/settings`) |
| R2b | the two write-only `INSERT INTO homepage_sections` seeds in `networks::create_network` and `directories::create_directory` | no reader existed after R2, so the table was **write-only** | removes pointless writes to a dead table |
| R3 | `monetization::{update_business_categories, list_business_categories}` + `UpdateCategoriesRequest` | not in `routes.rs`; no caller | duplicate of `category_system::{get_business_categories, set_business_categories}`, routed at `routes.rs:640-642` on the same path `/businesses/:id/categories` |

Post-sweep verification: `grep -rn "homepage_sections|HomepageSection|admin_list_directories|update_business_categories|list_business_categories|UpdateCategoriesRequest" src` → **0 hits** (outside migrations).

## 3. The rest of the "unreferenced" surface is NOT dead code — do not delete

A symbol scan (pub fns whose name appears only at its own definition) reported **63** candidates
before the sweep and **54** after. The remaining 54 are **unwired features / groundwork** for
open board cards, not cruft. Deleting them would delete planned capability:

* `coreswift.rs` (`push_loyalty_business`, `remove_contact_tag`, `add_to_claimed_list`) — native
  CoreSwift sync groundwork (B67/B111/B66).
* `entitlements.rs` (`require_reviews`, `require_crm`, `require_email`, `require_analytics`,
  `require_api_access`, `require_import_export`, `require_call_tracking`,
  `require_custom_branding`) — plan-tier gates that B113 requires be enforced server-side.
* `networks.rs` (`create/get/update/delete_network`, `list_network_directories`,
  `get/update_network_branding`) — the "unrouted networks create/manage" gap (B117 §F); B84 sellable standard.
* `business_articles.rs` (`list/update/delete/generate_article`, `generate_weekly`,
  `serve_article`, `track_article_event`) — the sponsored/SEO article system (B117 gap);
  the `business_articles` **table** is live (used by `subfolder`, `articles_feed`, `blog_seo`,
  `network_admin`, `entity_export`).
* `scraper.rs::run_scraper`, `import_export.rs::enrich_business_from_sources` — enrichment
  (B79/B83).
* `public.rs` public-page CRUD + `public_pages.rs::directory_landing_pages` — half-wired
  landing/public-pages feature (table `public_pages` is read by routed
  `public::list_directory_public_pages`, but create/update/delete are unrouted).
* `visitors.rs` (`get_visitor_summary`, `category_visitor_summary`, `export_visitors`),
  `monetization.rs` list helpers, `directories.rs::{categories_bulk_move, categories_bulk_delete}`,
  `call_tracking.rs::directory_phone_numbers`, `api_complete.rs` (`check_rate_limit`,
  `dispatch_webhook_event`), `host_resolver.rs::resolve_host`, `zaarhub_cities.rs::get_city`,
  `template_engine.rs::register_template`, `business_types.rs::as_json`,
  `auth/middleware.rs::auth_middleware`, `tenant_scope.rs::assert_transfer_party`,
  `email.rs::{get_directory_signature, append_signature}`, `automation.rs::record_event` —
  each either has a routed equivalent at a different path or is intended-wiring groundwork.

Full list: `/opt/swift/audits/B101-20261003/unreferenced-after.txt`.

## 4. Not touched, deliberately

* `src/migrations-legacy/` (124 superseded files) — retained by design; the install path is
  `000_baseline_live_schema.sql`. Never edit an applied file; a real change is a new file.
* `tag_automation.rs::execute_voucher_action` (POSTs to `localhost:8083` IncentiveSwift) —
  **REMOVED 2026-10-04 (t_8684ea3e)**. It was left here because a GET probe showed the route
  answered `405` (i.e. it existed) and loyalty-is-native-MD was a design question. That question
  was answered at source: IncentiveSwift retired the route (commit `43d3ce8a`) — it now answers a
  bare 0-byte `404`. The arm and the function are deleted, and migration
  `158_tag_rules_drop_issue_voucher_action.sql` drops `'issue_voucher'` from the
  `tag_rules_action_type_check` constraint, so the value can no longer be stored either. A row
  carrying it (impossible: `tag_rules` has never held a row) would fall to the `unknown_action` arm.
* Provider stub `allow(dead_code)` markers in `providers/eventbrite.rs` / `articles_feed.rs` —
  intentional.

## 5. Follow-up (not done here)

The `homepage_sections` **table** is now orphaned (no writer, no reader). Dropping it is a
destructive schema change and belongs in its own migration task, not a code-cleanup card.
Recommend a future `1xx_drop_homepage_sections.sql` once confirmed unneeded.
