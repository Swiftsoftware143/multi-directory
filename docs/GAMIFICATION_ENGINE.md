# Gamification Engine — Blueprint

## Overview
Dual-sided earning system where **visitors** earn 1% back in ZaarCash on purchases,
and **businesses/suppliers** earn ZaarCash through **engagement activities** —
keeping profiles sharp, responding to customers, and promoting the directory.

---

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

## Implementation Order

1. **Profile completion scoring** (MD migration + endpoint + recalculation trigger)
2. **Review response tracking** (add columns + response endpoint + ZC award)
3. **Business referral codes** (table + generate endpoint + public landing + click tracking)
4. **Business engagement ZC log** (table + endpoints)
5. **Supplier certifications** (table + upload + admin verify)
6. **IS badge triggers** (hooks from MD → IS on each engagement event)
7. **Admin engagement dashboard** (super admin view)

---

## Cost Estimate

At current rates (100 ZC = $1), maximum theoretical cost per business per year:
- Profile: $5.00 (one-time)
- Review responses: $1.00 per review answered
- Referral signups: $2.00 per signup
- Certs: $0.50 per cert (one-time)

**Worst case for a highly engaged business: ~$15-20/year in ZC rewards.**
At 1000 businesses = ~$15,000-20,000/year — funded by directory subscriptions.
