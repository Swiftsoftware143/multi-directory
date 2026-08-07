-- 029_zaarhub_claims: Claim Offers & Redemption Tables
-- Phase 5 — Business deal claim/redemption flow for ZaarHub

CREATE TABLE IF NOT EXISTS claim_offers (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    listing_id        UUID NOT NULL REFERENCES business_listings(id) ON DELETE CASCADE,
    offer_type        VARCHAR(50) NOT NULL DEFAULT 'promo_code',
    offer_title       VARCHAR(255) NOT NULL,
    offer_description TEXT,
    promo_code        VARCHAR(100),
    coupon_image_url  TEXT,
    redemption_url    TEXT,
    redemption_phone  VARCHAR(50),
    discount_value    VARCHAR(100),
    expires_at        TIMESTAMPTZ,
    terms_conditions  TEXT,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    max_claims        INTEGER,
    current_claims    INTEGER NOT NULL DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_claim_offers_listing ON claim_offers(listing_id);
CREATE INDEX idx_claim_offers_active ON claim_offers(is_active) WHERE is_active = true;
CREATE INDEX idx_claim_offers_type ON claim_offers(offer_type);

CREATE TABLE IF NOT EXISTS offer_claims (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    offer_id           UUID NOT NULL REFERENCES claim_offers(id) ON DELETE CASCADE,
    visitor_id         VARCHAR(255) NOT NULL,
    email              VARCHAR(255),
    phone              VARCHAR(50),
    promo_code_revealed VARCHAR(100),
    claimed_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    redeemed           BOOLEAN NOT NULL DEFAULT false,
    redeemed_at        TIMESTAMPTZ,
    ip_address         VARCHAR(50),
    user_agent         TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_offer_claims_offer ON offer_claims(offer_id);
CREATE INDEX idx_offer_claims_visitor ON offer_claims(visitor_id);
CREATE INDEX idx_offer_claims_redeemed ON offer_claims(redeemed) WHERE redeemed = false;
