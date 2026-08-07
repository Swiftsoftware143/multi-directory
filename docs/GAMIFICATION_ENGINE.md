# Gamification Engine — Blueprint

> ⚠️ **BLUEPRINT ONLY — NOT IMPLEMENTED.** This document describes planned future functionality. None of the endpoints or database tables described here exist in the codebase. See `ARCHITECTURE.md` for current system design.

## Overview
Dual-sided earning system where **visitors** earn 1% back in ZaarCash on purchases,
and **businesses/suppliers** earn ZaarCash through **engagement activities** —
keeping profiles sharp, responding to customers, and promoting the directory.

### Business Model: Paid Loyalty Program
Businesses **pay a monthly subscription** to participate in the loyalty program.
Their subscription funds the ZaarCash they issue to customers — no out-of-pocket
loss for the directory, no risk of businesses being drained.

| Plan | Monthly | ZC Monthly Pool | Max Monthly ZC | Annual Cost |
|------|---------|-----------------|----------------|-------------|
| **Starter** | $19/mo | 2,000 ZC ($20 value) | 2,000 ZC issued to customers | $228 |
| **Standard** | $49/mo | 6,000 ZC ($60 value) | 6,000 ZC issued to customers | $588 |
| **Premium** | $99/mo | 15,000 ZC ($150 value) | 15,000 ZC issued to customers | $1,188 |

**Pool mechanics:**
- Monthly ZC pool refreshes on the 1st of each month
- If pool runs dry, 2 options per directory: (a) earning pauses until next refill, or (b) business pays overage at 1:1 ZC-to-cents
- Pool can roll over for 1 month max (unused ZC doesn't accumulate forever)
- The 10% bill cap still applies — how the ZC pool translates to actual cost:
  - **Starter pool (2,000 ZC)** supports $2,000 in customer spend/month (at 1% cashback)
  - **Standard pool (6,000 ZC)** supports $6,000 in customer spend/month
  - **Premium pool (15,000 ZC)** supports $15,000 in customer spend/month
- Business pays $19 and gets to issue $20 worth of ZaarCash — the math stays slightly positive or break-even for the directory

---

## Database Migrations (IncentiveSwift)

### 0. loyalty_plans (IS)
Business subscription tiers for loyalty program access.

```sql
CREATE TABLE loyalty_plans (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name              TEXT NOT NULL,            -- "Starter", "Standard", "Premium"
    slug              TEXT NOT NULL UNIQUE,     -- "starter", "standard", "premium"
    monthly_price     INT NOT NULL,             -- cents (1900 = $19.00)
    monthly_zc_pool   INT NOT NULL,             -- ZC pool per month
    overage_rate      INT NOT NULL DEFAULT 1,   -- cents per ZC overage (1 = 100 ZC per $1)
    features          TEXT[],                   -- ["scanner","offers","referrals","badges"]
    is_active         BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO loyalty_plans (name, slug, monthly_price, monthly_zc_pool, features) VALUES
  ('Starter',  'starter',  1900, 2000,  ARRAY['scanner','basic_offers']),
  ('Standard', 'standard', 4900, 6000,  ARRAY['scanner','offers','referrals','badges']),
  ('Premium',  'premium',  9900, 15000, ARRAY['scanner','offers','referrals','badges','analytics','featured']);
```

### 0a. accounts loyalty columns (IS)
```sql
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS stripe_customer_id   TEXT;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS stripe_subscription_id TEXT;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS loyalty_plan         TEXT;  -- 'starter','standard','premium'
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS loyalty_plan_status  TEXT NOT NULL DEFAULT 'inactive';  -- active, past_due, canceled, trial
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS zc_pool_remaining    INT NOT NULL DEFAULT 0;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS zc_pool_total        INT NOT NULL DEFAULT 0;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS pool_reset_date      DATE;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS trial_ends_at         TIMESTAMPTZ;
```

### 0b. purchase_verify changes (IS)
On each purchase verify:
1. Check `loyalty_plan_status = 'active'` — reject if not paid
2. Check `zc_pool_remaining >= credit_amount` — if not:
   - Option A (default): reject with "business ZC pool exhausted"
   - Option B (overage): deduct from pool, log overage for end-of-month billing
3. Deduct `credit_amount` from `zc_pool_remaining`
4. Monthly cron to refill `zc_pool_remaining = zc_pool_total` on 1st of month

## Database Migrations (MultiDirectory)

### 1. profile_completion_scores
Track how complete each business profile is — auto-award ZC on 100%.

```sql
CREATE TABLE profile_completion_scores (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id     UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id    UUID REFERENCES directories(id),
    score_pct       INT NOT NULL DEFAULT 0,       -- 0-100
    fields_missing  TEXT[] DEFAULT '{}',          -- ["phone","description","images"]
    fields_complete TEXT[] DEFAULT '{}',
    awarded_zc      INT NOT NULL DEFAULT 0,       -- ZC awarded on hitting 100%
    first_100_at    TIMESTAMPTZ,                  -- when they first hit 100%
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_pcs_business ON profile_completion_scores(business_id);
```

**Fields scored (business):** name, description, category, address, phone, email, website, images (≥1), hours
**Fields scored (supplier):** above + supplier_fields jsonb keys (certifications, inventory, MOQ, lead_time)

### 2. business_referral_codes
Businesses get a unique code to share on their storefront/website.

```sql
CREATE TABLE business_referral_codes (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id   UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    code          TEXT NOT NULL UNIQUE,           -- e.g. "TONYS-PIZZA-ZH"
    directory_id  UUID REFERENCES directories(id),
    total_clicks  INT NOT NULL DEFAULT 0,
    total_signups INT NOT NULL DEFAULT 0,
    is_active     BOOLEAN NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_brc_code ON business_referral_codes(code);
```

### 3. review_responses
Track if/when a business replied to a review (engagement scoring).

```sql
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS business_response      TEXT;
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS business_responded_at  TIMESTAMPTZ;
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS response_zc_awarded    BOOLEAN NOT NULL DEFAULT false;
```

### 4. business_engagement_zc
Audit log of engagement ZC awards (profile completion, review response, referrals).

```sql
CREATE TABLE business_engagement_zc (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id     UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    directory_id    UUID REFERENCES directories(id),
    activity_type   TEXT NOT NULL,  -- profile_complete, review_response, referral_click, referral_signup, inventory_update, cert_update, rkq_response
    description     TEXT,
    zc_earned       INT NOT NULL,
    reference_id    UUID,           -- points to the source row (review id, referral code id, etc.)
    is_awarded      BOOLEAN NOT NULL DEFAULT false,
    awarded_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_bez_business ON business_engagement_zc(business_id);
CREATE INDEX idx_bez_type ON business_engagement_zc(activity_type);
```

### 5. supplier_certifications
Track supplier compliance docs for earning.

```sql
CREATE TABLE supplier_certifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id     UUID NOT NULL REFERENCES businesses(id) ON DELETE CASCADE,
    cert_type       TEXT NOT NULL,  -- organic, fair_trade, safety, gmp, kosher, halal
    cert_name       TEXT NOT NULL,  -- "USDA Organic", "Fair Trade Certified"
    file_url        TEXT,
    issued_by       TEXT,
    issued_date     DATE,
    expiry_date     DATE,
    is_verified     BOOLEAN NOT NULL DEFAULT false,
    zc_awarded      BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_sc_business ON supplier_certifications(business_id);
```

---

## API Endpoints (MultiDirectory)

### Profile Completion
```
GET  /api/v1/business/profile/score        → returns score_pct, missing fields, ZC awarded
POST /api/v1/business/profile/claim-zc     → claims ZC for profile completion (one-time)
```
- On profile update → recalculate score_pct asynchronously
- On reaching 100% → auto-flag for ZC claim (never auto-award — business must opt in)

### Business Referrals
```
GET  /api/v1/business/referral/code        → returns referral code + stats
POST /api/v1/business/referral/regenerate  → new code (invalidates old)
GET  /api/v1/directory/referral/:code      → PUBLIC — landing page with directory info, tracks click
POST /api/v1/directory/referral/:code/signup → PUBLIC — visitor signs up via referral, awards ZC
```

### Review Responses
```
POST /api/v1/business/reviews/:id/respond  → submit business response, award ZC if first response to this review
GET  /api/v1/business/reviews/pending      → reviews needing response (no business_response yet)
```
- Award ZC rule: first response per review = bonus ZC. Subsequent edits = no additional ZC.

### Supplier Certifications
```
POST   /api/v1/supplier/certifications          → upload new cert
GET    /api/v1/supplier/certifications           → list certs + ZC awarded status
DELETE /api/v1/supplier/certifications/:id       → remove
POST   /api/v1/supplier/certifications/:id/verify → admin-only: verify cert, award ZC
```

### Engagement Dashboard
```
GET /api/v1/business/engagement                 → all engagement ZC earned (breakdown by type)
GET /api/v1/admin/engagement/:directory_slug    → super admin view — all businesses ranked by engagement
```

---

## ZC Award Table

| Activity | ZC Awarded | Rule |
|----------|-----------|------|
| Profile 100% complete | 500 ZC ($5.00) | One-time, business must opt in |
| First review response | 100 ZC ($1.00) | Per review, first response only |
| Referral click | 10 ZC ($0.10) | Per unique click (IP-based dedup daily) |
| Referral signup (visitor) | 200 ZC ($2.00) | Visitor signed up via referral code |
| Cert upload (each) | 50 ZC ($0.50) | One-time per cert, admin verified |
| Inventory update | 25 ZC ($0.25) | Per update, max 1/day |
| RFQ response < 24h | 150 ZC ($1.50) | Per RFQ, first to respond |

---

## Badge System (IncentiveSwift)

IS already has `loyalty_badges` routes. We need to extend the badge types:

### New Badge Types
```sql
INSERT INTO loyalty_programs (name, slug, currency_name, currency_icon, currency_color)
VALUES ('Business Engagement', 'business-engagement', 'Engagement Points', '🏆', '#f27f2f');
```

Badges awarded automatically when thresholds hit:

| Badge | Condition | Icon |
|-------|-----------|------|
| Profile Master | Profile reached 100% | ✅ |
| Quick Responder | 10+ review responses | 💬 |
| Community Builder | 3+ referral signups | 🌐 |
| Certified Supplier | 3+ verified certifications | 📜 |
| Top Engager | 1000+ total engagement ZC | 🏆 |
| Inventory Hero | 30+ inventory updates in 30 days | 📦 |
| RFQ Speedster | 5+ RFQ responses under 12h | ⚡ |

---

## Paid Plan Flow (Stripe Integration)

### Enrollment flow
```
Business clicks "Join Loyalty" in portal
  └─ MD checks: is business a paid subscriber?
       ├─ NO → redirect to Stripe checkout
       │        └─ Stripe creates subscription (monthly)
       │        └─ IS webhook: POST /api/v1/webhooks/stripe
       │             └─ Sets loyalty_plan_status='active', assigns loyalty_plan, sets zc_pool
       │             └─ Redirects back to portal → enrollment complete
       └─ YES → enrolls in IS loyalty program immediately
```

### Stripe Webhook Events
```
POST /api/v1/webhooks/stripe (in IS)
  Events handled:
  - checkout.session.completed → activate subscription, set pool
  - invoice.paid → reset monthly pool
  - invoice.payment_failed → set loyalty_plan_status='past_due', pause earning
  - customer.subscription.deleted → set loyalty_plan_status='canceled', revoke enrollment
```

### API Endpoints
```
# MD: Check subscription status
GET  /api/v1/business/loyalty/status
     → { enrolled: bool, plan: "starter", zc_pool_remaining: 1850, pool_reset_date: "2026-08-01" }

# MD: Start checkout flow
POST /api/v1/business/loyalty/subscribe
     → { checkout_url: "https://checkout.stripe.com/..." }

# MD: Cancel subscription
POST /api/v1/business/loyalty/cancel
     → { canceled_at: "...", pool_expires: "..." }

# IS: Stripe webhook
POST /api/v1/webhooks/stripe
     (Stripe-signed, handles all subscription lifecycle events)
```

## Implementation Order

0. **Paid subscription gating** (IS: loyalty_plans table, accounts columns, Stripe webhook, purchase_verify gate)
1. **Profile completion scoring** (MD migration + endpoint + recalculation trigger)
2. **Review response tracking** (add columns + response endpoint + ZC award)
3. **Business referral codes** (table + generate endpoint + public landing + click tracking)
4. **Business engagement ZC log** (table + endpoints)
5. **Supplier certifications** (table + upload + admin verify)
6. **IS badge triggers** (hooks from MD → IS on each engagement event)
7. **Admin engagement dashboard** (super admin view)

---

## Cost Estimate

Business pays monthly subscription — the ZC pool is included.
Directory has zero out-of-pocket for loyalty rewards. The 10% bill cap
ensures the pool stretches across actual customer spend:
- Starter: $19/mo → covers $2,000 in monthly customer spend
- Standard: $49/mo → covers $6,000 in monthly customer spend  
- Premium: $99/mo → covers $15,000 in monthly customer spend

**Directory revenue at scale:**
- 100 businesses on Starter = $1,900/mo MRR
- 100 on Standard = $4,900/mo MRR  
- Mix: 200 Starter + 50 Standard + 10 Premium = $7,240/mo MRR

**Key win:** The loyalty program becomes a **profit center** for the directory,
not a cost center. Businesses pay for customer retention tools they'd pay for anyway.

---

## Immediate Next Step: Gate Enrollment Behind Paid Subscription

Before building profile scoring, referrals, or certs — gate the existing
`POST /loyalty/enroll` behind a subscription check. No paid plan = no enrollment.

Steps:
1. Create `loyalty_plans` table in IS with 3 tiers
2. Add Stripe columns to `accounts` table
3. Wire Stripe checkout → webhook → activation in IS
4. Update `purchase_verify` to check `loyalty_plan_status = 'active'`
5. Update `POST /loyalty/enroll` in MD to check subscription status first
6. Add `GET /business/loyalty/status` + `POST /business/loyalty/subscribe` endpoints
7. Update portal CTAs: "Enroll Now" → "Start Free Trial" → Stripe checkout
