# .openclaw/rules.md — Multi-Directory Agent Rules
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
- Admin credentials: `swiftsoftware143@yahoo.com` / `SwiftAdmin2026!`
- After EVERY deploy: verify admin login works
- For SaaS apps: `https://admin.{domain}/` must accept these credentials
- For Multi-Directory: `https://directory.swiftsoftware.net/admin` must accept these credentials
- If admin login returns 401/422/500 — deployment is BROKEN, roll back immediately
- This applies across ALL 7 apps — no exceptions
