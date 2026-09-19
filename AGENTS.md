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

---

## OpenClaw-era rules (carried over 2026-09-19)

> Source: `.openclaw/rules.md` (the OpenClaw-era bot rules). Hermes Agent reads
> `AGENTS.md`, so this content is mirrored here to stop it going unseen.

# .openclaw/rules.md

## RULES PROTECTION — READ THIS FIRST

**These rules are READ-ONLY.** Do NOT modify, rewrite, or "improve" this file unless the CEO Bot or David explicitly instructs you to do so. This file exists to guard against regression — editing it defeats its purpose.

**Before declaring any task complete:** Re-read these rules and confirm your changes satisfy every applicable rule. If you skip a rule, the task is NOT done.

---

 — Multi-Directory Agent Rules
#
# This file is read by OpenClaw on EVERY context load for this repo.
# Multi-Directory is a listing/search platform (not a traditional SaaS).
# It does NOT have user accounts, register/login, or subscription plans.

## CRITICAL — NEVER VIOLATE THESE

### 1. No Direct VPS Edits
- NEVER edit files directly on the VPS without committing
- Script: `git add → git commit → git push → deploy`
- If it's not pushed, it doesn't exist

### 2. Workspace Hygiene
- Delete ALL temp scripts (`*.sh`, `*.py`, `*.json` test payloads) before `git commit`
- NEVER commit `/tmp/` files, `.cargo/`, `target/`, `.bak` files
- Run `git status` before every commit — if you see anything that isn't `src/`, `Cargo.toml`, `Cargo.lock`, or config, STOP

### 3. Full Feature Journey Required
- A feature is NOT done until backend + admin UI are both connected
- Verify: admin page loads → feature is visible → API returns correct data
- "It works on localhost" is not sufficient — smoke test through `https://directory.swiftsoftware.net`

### 4. Directory-Specific Architecture
- This is a **listing/search engine** — not a SaaS with user subscriptions
- There is NO `plans` table, NO `register` endpoint, NO upgrade gating
- The "tenant" is the directory itself — no multi-tenant user accounts
- Business listing signups (if enabled) go through `/api/business/signup` — NOT through user auth

### 5. Build Pipeline
- ALWAYS use `/opt/swift/build-lock.sh multi-directory cargo build --release` — NEVER raw `cargo build`
- `cargo check` must pass with zero errors before building
- `systemctl restart multi-directory` after every deploy
- Smoke test the app after restart

### 6. Routing
- Main domain: `https://directory.swiftsoftware.net` — serves the directory listing + admin
- NO `app.*` or `admin.*` subdomains — it's a single-site architecture
- Admin panel is at `https://directory.swiftsoftware.net/admin`

### 7. No Dead Endpoints
- Every API route in `main.rs`/`routes.rs` must have a corresponding frontend caller
- If a route has no frontend, either add the UI or document it as `// INTERNAL`

### 8. Git Protocol
- `git pull` before starting any work
- Commit after every meaningful change
- Push after every commit
- Feature branches for multi-commit work: `ceobot/feature-name`

---

## Multi-Directory Specifics

### What It Is
- A local business/community directory with AI-powered search
- AEO (Answer Engine Optimization) scoring for listed businesses
- No user accounts — public-facing only
- Admin panel for directory management

### What It Is NOT
- NOT a SaaS platform with subscription plans
- NOT a multi-tenant app with user registration
- NOT gated by plan limits or 402 upgrade prompts

### When Adding Business Signup (if requested)
- Endpoint: `POST /api/business/signup`
- This is for businesses to register on the directory — NOT for user accounts
- No plan assignment needed — all businesses get the same listing

---

## Deployment Checklist Reference
After EVERY deploy, run through the appropriate checklist:
- Core checks: Source Control → Build → Git Sync → Feature Verification → Nginx → Heartbeat
- SKIP: Signup flow, 402 gating, plan management (not applicable)
- Reference: `memory/deployment-checklist.md`


### 9. Admin Login — NEVER BREAK
- Admin credentials: `swiftsoftware143@yahoo.com` / `<REDACTED-ROTATED-2026-09-16>`
- After EVERY deploy: verify admin login works
- For SaaS apps: `https://admin.{domain}/` must accept these credentials
- For Multi-Directory: `https://directory.swiftsoftware.net/admin` must accept these credentials
- If admin login returns 401/422/500 — deployment is BROKEN, roll back immediately
- This applies across ALL 7 apps — no exceptions
