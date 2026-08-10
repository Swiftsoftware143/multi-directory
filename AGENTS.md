# AGENTS.md — Vibe Engineering Rules for AI Agents

## Rust Guardrails (MANDATORY)
- **Zero unsafe blocks** unless explicitly approved by the Lead Architect
- **Zero .unwrap() or .expect()** in non-test production code — use `thiserror`/`anyhow`
- **All async state must implement Send + Sync**
- **Parameterized SQL only** — use `sqlx::query_as!` for compile-time validation
- **Secrets in env vars only** — never hardcoded
- **cargo fmt** before commit

## Verification Sequence (NON-NEGOTIABLE)
After ANY code change:
1. `cargo check` — syntax + borrow checker. Read stderr. Fix. Repeat until clean.
2. `cargo test` — all tests must pass
3. `cargo clippy -- -D warnings` — zero warnings tolerated
4. `cargo fmt -- --check` — formatting must be consistent

## Self-Correction Loop
- Compiler error → read diagnostic → understand → fix → re-compile
- Test failure → fix logic → re-run
- Clippy warning → clean up → re-run
- **NEVER paste errors to a human. FIX THEM.**
- 3 attempts max, then escalate with evidence of what you tried.

## Hermes Delegation Pattern
For complex feature implementation:
1. Draft trait signatures and types FIRST
2. Run `cargo check` to validate types before writing method bodies
3. Then implement method logic — iterate with check/test/clippy
4. Re-run full verification before declaring done

## Build Lock Protocol
- ALWAYS use `/opt/swift/build-lock.sh <app> <command>`
- Never raw `cargo build --release` on shared repos
- Exit 2 = another bot building → wait 30s, retry once
- Stale lock >30min: clear and proceed

## Post-Deploy Smoke Test
- `curl -s -o /dev/null -w "%{http_code}" <domain>` must return 200

## Project File Architecture
```
src/auth/handlers.rs
src/auth/middleware.rs
src/auth/mod.rs
src/auth/models.rs
src/config.rs
src/coreswift.rs
src/db.rs
src/email.rs
src/error.rs
src/handlers/admin.rs
src/handlers/analytics.rs
src/handlers/answer_first.rs
src/handlers/api_complete.rs
src/handlers/articles_feed.rs
src/handlers/auth_handler.rs
src/handlers/automation.rs
src/handlers/b2b.rs
src/handlers/b2b_ssr.rs
src/handlers/blog.rs
src/handlers/blog_features.rs
src/handlers/blog_generator.rs
src/handlers/blog_pages.rs
src/handlers/blog_qa.rs
src/handlers/blog_seo.rs
src/handlers/booking_page.rs
src/handlers/bookings.rs
src/handlers/branding.rs
src/handlers/business_articles.rs
src/handlers/business_dashboard.rs
src/handlers/businesses.rs
src/handlers/call_tracking.rs
src/handlers/categories.rs
src/handlers/category_system.rs
src/handlers/checkout_handler.rs
src/handlers/connected_services.rs
src/handlers/contact_intelligence.rs
src/handlers/content_queue.rs
src/handlers/content_research.rs
src/handlers/content_seo.rs
src/handlers/coop.rs
```
