# Multi-Directory Admin Guide

> **Last Updated:** 2026-07-18 23:40 EDT

---

## Table of Contents
1. [Claim → Pipeline Deal Creation](#1-claim--pipeline-deal-creation)
2. [Auto-Fetch Business Images on Claim](#2-auto-fetch-business-images-on-claim)
3. [CRM Pipelines](#3-crm-pipelines)
4. [Place Enrichment](#4-place-enrichment)

---

## 1. Claim → Pipeline Deal Creation

**File:** `src/handlers/visitors.rs` — `create_claim_deal()` (line ~665)

**How it works:**
When a visitor claims a business via `POST /api/v1/businesses/:id/claim`, the backend automatically creates a **CRM deal record** in the directory's default pipeline.

| Stage | Detail |
|-------|--------|
| **Deal Title** | `"{Business Name} - Claimed ({City})"` |
| **Initial Stage** | First stage of the default pipeline |
| **Status** | `open` |
| **Pipeline Selection** | Prefers the directory-specific pipeline; falls back to global pipeline marked `default_pipeline = true` |

**Pipeline auto-advance:**
- On **booking** (plan slot booking): deal advances from current stage → `Qualified` (wired in `bookings.rs`)
- Additional auto-advance rules can be added per-pipeline in `bookings.rs`

**Configuration:**
- At least one CRM pipeline must exist (auto-seeded or created via admin UI)
- Set `default_pipeline = true` on the Sales Pipeline to auto-attach new directories
- SQL: `UPDATE crm_pipelines SET default_pipeline = true WHERE name = 'Sales Pipeline';`

---

## 2. Auto-Fetch Business Images on Claim

**File:** `src/handlers/visitors.rs` — `fetch_business_images_on_claim()` (bottom of file)

**How it works:**
Triggers as a **fire-and-forget background task** immediately after a business is claimed.

1. Reads business **name** + **city** from DB
2. Calls **Google Places API** `findplacefromtext` with the name+city query
3. Gets the matching `place_id`
4. Calls **Google Places API** `place/details` with `fields=photos`
5. Extracts `photo_reference` strings, builds full image URLs
6. Stores URLs in `businesses.images` (jsonb array)
7. Overwrites existing images with fresh data from Google

**Non-blocking:** The claim endpoint returns immediately — images populate in the background within 2-5 seconds.

**Requirements:**
- `GOOGLE_PLACES_API_KEY` must be set in `.env` or environment
- No changes needed per-directory — applies to all claimed businesses

---

## 3. CRM Pipelines

**Table:** `crm_pipelines`
**Table:** `crm_deal_records`

**Schema:**
```
crm_pipelines (
  id            UUID PRIMARY KEY,
  name          VARCHAR(255),       -- e.g. "Sales Pipeline"
  stages        JSONB,              -- ["Lead", "Qualified", "Proposal", "Closed Won"]
  directory_id  UUID REFERENCES directories(id) (nullable)
  default_pipeline BOOLEAN DEFAULT false
)

crm_deal_records (
  id            UUID PRIMARY KEY,
  pipeline_id   UUID REFERENCES crm_pipelines(id),
  business_id   UUID REFERENCES businesses(id),
  title         VARCHAR(255),
  stage         VARCHAR(100),       -- current pipeline stage
  status        VARCHAR(50),        -- open / closed_won / closed_lost
  directory_id  UUID REFERENCES directories(id)
)
```

---

## 4. Place Enrichment

**Endpoint:** `POST /api/v1/enrich/places`

**File:** `src/handlers/data_company.rs` — `enrich_places()` (line ~550)

Used by the data company / directory admin to manually enrich a business listing with Google Places data (address, phone, website, lat/lng, rating). Also logs to `data_enrichment_logs` table.

| Field | Source |
|-------|--------|
| address | Google Places `formatted_address` |
| phone | Google Places `formatted_phone_number` |
| website | Google Places `website` |
| lat/lng | Google Places `geometry.location` |
| rating | Google Places `rating` |

Only fills in fields that are currently empty in the DB (won't overwrite existing data).

---

## Changelog

| Date | Change |
|------|--------|
| 2026-07-18 | Initial guide — claim pipeline, auto-fetch images, CRM pipelines, enrichment |
