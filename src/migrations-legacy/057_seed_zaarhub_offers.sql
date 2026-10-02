-- 030_seed_zaarhub_offers: Seed sample claim offers for ZaarHub
-- Phase 5 — Sample offers for high-rated businesses

-- Get some business IDs to seed against
-- Run this after 027 + 028 migrations have been applied

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'promo_code',
    '20% Off First Service',
    'New customers get 20% off their first service booking. Must mention this offer when scheduling.',
    'ZAAR20',
    '20% off',
    NULL,
    NULL,
    '2026-12-31 23:59:59+00',
    'Valid for new customers only. Cannot be combined with other offers. One use per customer.',
    500,
    true
FROM business_listings bl
WHERE bl.business_name ILIKE '%plumb%' OR bl.business_name ILIKE '%roof%'
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'promo_code',
    'Free Consultation',
    'Book a free 30-minute consultation for any residential service. No obligation.',
    'FREECONSULT',
    'Free',
    NULL,
    NULL,
    '2026-10-31 23:59:59+00',
    'Available for residential customers within 20-mile radius. Appointment required.',
    200,
    true
FROM business_listings bl
WHERE bl.business_name ILIKE '%electric%' OR bl.business_name ILIKE '%handyman%'
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'promo_code',
    '$50 Off AC Tune-Up',
    'Keep your home cool this summer. $50 off any AC tune-up service.',
    'COOL50',
    '$50 off',
    NULL,
    NULL,
    '2026-09-30 23:59:59+00',
    'Valid for residential AC units only. One per household. Expires Sept 30, 2026.',
    300,
    true
FROM business_listings bl
WHERE bl.business_name ILIKE '%hvac%' OR bl.business_name ILIKE '%cool%' OR bl.business_name ILIKE '%air%'
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'coupon',
    'Buy One Get One 50% Off',
    'Buy any entree and get a second entree of equal or lesser value at 50% off. Dine-in only.',
    'BOGO50',
    '50% off 2nd item',
    NULL,
    NULL,
    '2026-11-15 23:59:59+00',
    'Valid Monday-Thursday. Not valid on holidays. Dine-in only.',
    1000,
    true
FROM business_listings bl
WHERE bl.category ILIKE '%restaurant%' OR bl.category ILIKE '%food%' OR bl.category ILIKE '%dining%'
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'promo_code',
    '10% Off For Locals',
    'Show your local ID and get 10% off your entire purchase. Supporting our community!',
    'LOCAL10',
    '10% off',
    NULL,
    NULL,
    '2026-12-31 23:59:59+00',
    'Must show proof of local residency. Cannot be combined with other discounts.',
    NULL,
    true
FROM business_listings bl
WHERE bl.rating >= 4.0 AND bl.review_count >= 5
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'link',
    'Exclusive Web Deal — 15% Off',
    'Click to claim your exclusive online-only discount. Automatically applied at checkout.',
    NULL,
    '15% off',
    'https://example.com/deals/zaarhub-special',
    NULL,
    '2026-12-31 23:59:59+00',
    'Online orders only. One use per customer.',
    500,
    true
FROM business_listings bl
WHERE bl.website IS NOT NULL AND bl.rating >= 4.0
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'phone_deal',
    'Call Now For Special Pricing',
    'Mention ZaarHub when you call and receive a special discounted rate on your first service.',
    NULL,
    'Special rate',
    NULL,
    bl.phone,
    '2026-12-31 23:59:59+00',
    'First-time customers only. Must mention ZaarHub during booking.',
    100,
    true
FROM business_listings bl
WHERE bl.phone IS NOT NULL AND bl.is_featured = true
LIMIT 1;

INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'promo_code',
    'Spring Cleaning Special',
    'Get your home sparkling clean with 25% off deep cleaning services.',
    'SPRING25',
    '25% off',
    NULL,
    NULL,
    '2026-08-31 23:59:59+00',
    'Valid for deep cleaning services over $200. Not valid with other offers. Appointment required.',
    150,
    true
FROM business_listings bl
WHERE bl.category ILIKE '%clean%' OR bl.description ILIKE '%clean%'
LIMIT 1;

-- Fallback: Seed offers for any businesses with ratings if specific category ones didn't match
INSERT INTO claim_offers (listing_id, offer_type, offer_title, offer_description, promo_code, discount_value, redemption_url, redemption_phone, expires_at, terms_conditions, max_claims, is_active)
SELECT
    bl.id,
    'promo_code',
    '$25 Off Your First Order',
    'Welcome to the neighborhood! Take $25 off your first order of $100 or more.',
    'WELCOME25',
    '$25 off',
    NULL,
    NULL,
    '2026-12-31 23:59:59+00',
    'Minimum purchase $100. New customers only. One use per household.',
    500,
    true
FROM business_listings bl
WHERE bl.rating >= 3.5
  AND bl.id NOT IN (SELECT listing_id FROM claim_offers)
LIMIT 4;
