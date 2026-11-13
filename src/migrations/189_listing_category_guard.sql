-- 189_listing_category_guard.sql
--
-- A live listing with no category renders "X is a local business in <city>" and has no
-- category breadcrumb, so it loses the internal link path every other listing has.
-- 251 of 4,100 active listings were in that state (card t_e092976e).
--
-- This migration makes that state unreachable from both directions:
--   1. `business_type_category_map` is the ONE vocabulary mapping a Google Places type to a
--      directory category slug (data, not code, so it can be corrected without a rebuild).
--   2. `businesses_require_category()` runs BEFORE INSERT OR UPDATE: when a listing would go
--      live with category_id IS NULL it resolves the category from the Places types already
--      cached for that place; if nothing resolves, it refuses to publish the row
--      (is_active := false) instead of publishing a category-less page.

CREATE TABLE IF NOT EXISTS business_type_category_map (
    google_type   TEXT PRIMARY KEY,
    category_slug TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO business_type_category_map (google_type, category_slug) VALUES
    ('accountant', 'accountant'),
    ('accounting', 'accountant'),
    ('american_restaurant', 'american'),
    ('amusement_park', 'family-entertainment'),
    ('animal_hospital', 'veterinarian'),
    ('appliance_store', 'home-decor'),
    ('arborist', 'tree-service'),
    ('arcade', 'family-entertainment'),
    ('asian_fusion_restaurant', 'food-drink'),
    ('asian_restaurant', 'food-drink'),
    ('auto_body_shop', 'auto-repair'),
    ('auto_repair', 'auto-repair'),
    ('bagel_shop', 'breakfast'),
    ('bakery', 'bakeries-dessert'),
    ('bank', 'professional-services'),
    ('banquet_hall', 'special-events'),
    ('bar', 'bars-breweries'),
    ('bar_and_grill', 'bars-breweries'),
    ('barbecue_restaurant', 'american'),
    ('barber_shop', 'hair-salon'),
    ('beauty_salon', 'hair-salon'),
    ('bistro', 'french'),
    ('boat_charter', 'boat-charter'),
    ('boat_rental', 'boat-charter'),
    ('book_store', 'books-music-hobby'),
    ('bookkeeping', 'accountant'),
    ('bookstore', 'books-music-hobby'),
    ('boutique', 'boutiques-thrift'),
    ('bowling_alley', 'family-entertainment'),
    ('brazilian_restaurant', 'food-drink'),
    ('breakfast_restaurant', 'breakfast'),
    ('brewery', 'bars-breweries'),
    ('brunch_restaurant', 'breakfast'),
    ('buffet_restaurant', 'food-drink'),
    ('business_service', 'professional-services'),
    ('butcher', 'specialty-grocery-gourmet'),
    ('cafe', 'coffee-shop'),
    ('cake_shop', 'bakeries-dessert'),
    ('car_detailing', 'car-detailing-wash'),
    ('car_repair', 'auto-repair'),
    ('car_wash', 'car-detailing-wash'),
    ('caribbean_restaurant', 'food-drink'),
    ('carpenter', 'maintenance-handyman'),
    ('caterer', 'catering'),
    ('catering_service', 'catering'),
    ('chicken_restaurant', 'american'),
    ('child_care_agency', 'child-care-education'),
    ('chinese_restaurant', 'chinese'),
    ('chiropractor', 'chiropractor'),
    ('cleaning_service', 'cleaning-services'),
    ('clothing_store', 'boutiques-thrift'),
    ('cocktail_bar', 'bars-breweries'),
    ('coffee_roastery', 'coffee-shop'),
    ('coffee_shop', 'coffee-shop'),
    ('confectionery', 'bakeries-dessert'),
    ('construction_company', 'maintenance-handyman'),
    ('consultant', 'professional-services'),
    ('contractor', 'maintenance-handyman'),
    ('convenience_store', 'specialty-grocery-gourmet'),
    ('cuban_restaurant', 'food-drink'),
    ('day_spa', 'day-spa'),
    ('deli', 'food-drink'),
    ('dental_clinic', 'dentist'),
    ('dentist', 'dentist'),
    ('dessert_shop', 'bakeries-dessert'),
    ('diner', 'american'),
    ('doctor', 'medical-health'),
    ('dog_daycare', 'dog-daycare-boarding'),
    ('donut_shop', 'bakeries-dessert'),
    ('dry_cleaning', 'cleaning-services'),
    ('eastern_european_restaurant', 'food-drink'),
    ('electrician', 'electrician'),
    ('electronics_repair', 'electronics-repair-resale'),
    ('electronics_store', 'electronics-repair-resale'),
    ('european_restaurant', 'food-drink'),
    ('event_venue', 'special-events'),
    ('family_restaurant', 'american'),
    ('farmers_market', 'farmers-market'),
    ('fast_food_restaurant', 'food-drink'),
    ('financial_planner', 'professional-services'),
    ('fine_dining_restaurant', 'fine-dining'),
    ('fitness_center', 'fitness-studio'),
    ('florist', 'florists-garden-shops'),
    ('food_court', 'food-trucks'),
    ('food_truck', 'food-trucks'),
    ('french_restaurant', 'french'),
    ('furniture_store', 'home-decor'),
    ('garden_center', 'florists-garden-shops'),
    ('gardener', 'landscaping'),
    ('general_contractor', 'maintenance-handyman'),
    ('general_handyman', 'maintenance-handyman'),
    ('general_practitioner', 'medical-health'),
    ('german_restaurant', 'food-drink'),
    ('gift_shop', 'gifts-stationery'),
    ('greek_restaurant', 'mediterranean'),
    ('grocery_store', 'specialty-grocery-gourmet'),
    ('gym', 'fitness-studio'),
    ('hair_care', 'hair-salon'),
    ('hair_salon', 'hair-salon'),
    ('hamburger_restaurant', 'american'),
    ('handyman', 'maintenance-handyman'),
    ('hardware_store', 'home-decor'),
    ('health', 'health-wellness'),
    ('health_food_restaurant', 'food-drink'),
    ('hobby_shop', 'books-music-hobby'),
    ('home_builder', 'maintenance-handyman'),
    ('home_goods_store', 'home-decor'),
    ('home_improvement_store', 'home-decor'),
    ('hospital', 'medical-health'),
    ('house_cleaning', 'cleaning-services'),
    ('house_painter', 'maintenance-handyman'),
    ('hvac_contractor', 'hvac'),
    ('ice_cream_shop', 'bakeries-dessert'),
    ('indian_restaurant', 'indian'),
    ('insurance_agency', 'professional-services'),
    ('italian_restaurant', 'italian'),
    ('janitorial_service', 'cleaning-services'),
    ('japanese_restaurant', 'japanese'),
    ('jewelry_store', 'gifts-stationery'),
    ('korean_restaurant', 'food-drink'),
    ('landscape_designer', 'landscaping'),
    ('landscaper', 'landscaping'),
    ('landscaping', 'landscaping'),
    ('latin_american_restaurant', 'food-drink'),
    ('laundry', 'cleaning-services'),
    ('laundry_service', 'cleaning-services'),
    ('law_firm', 'professional-services'),
    ('lawyer', 'professional-services'),
    ('legal_service', 'professional-services'),
    ('liquor_store', 'specialty-grocery-gourmet'),
    ('locksmith', 'maintenance-handyman'),
    ('marina', 'local-attractions-tours'),
    ('market', 'specialty-grocery-gourmet'),
    ('marketing_agency', 'professional-services'),
    ('mattress_store', 'home-decor'),
    ('meal_delivery', 'food-drink'),
    ('meal_takeaway', 'food-drink'),
    ('mechanic', 'auto-repair'),
    ('medical_center', 'medical-health'),
    ('medical_clinic', 'medical-health'),
    ('medical_lab', 'medical-health'),
    ('medical_spa', 'medical-spa'),
    ('mediterranean_restaurant', 'mediterranean'),
    ('mexican_restaurant', 'mexican'),
    ('middle_eastern_restaurant', 'food-drink'),
    ('movie_theater', 'family-entertainment'),
    ('moving_company', 'maintenance-handyman'),
    ('museum', 'local-attractions-tours'),
    ('music_store', 'books-music-hobby'),
    ('nail_salon', 'personal-care'),
    ('night_club', 'live-music-bars'),
    ('oil_change', 'tire-oil-change'),
    ('orthodontist', 'dentist'),
    ('oyster_bar_restaurant', 'seafood'),
    ('painter', 'maintenance-handyman'),
    ('pastry_shop', 'bakeries-dessert'),
    ('personal_trainer', 'fitness-studio'),
    ('peruvian_restaurant', 'food-drink'),
    ('pet_boarding', 'dog-daycare-boarding'),
    ('pet_groomer', 'pet-groomer'),
    ('pharmacy', 'medical-health'),
    ('physical_therapist', 'physical-therapy'),
    ('physiotherapist', 'physical-therapy'),
    ('pilates_studio', 'fitness-studio'),
    ('pizza_restaurant', 'pizza'),
    ('plant_nursery', 'florists-garden-shops'),
    ('plumber', 'plumber'),
    ('pool_cleaning_service', 'pool-service'),
    ('pool_service', 'pool-service'),
    ('preschool', 'child-care-education'),
    ('pressure_washing_service', 'cleaning-services'),
    ('primary_school', 'child-care-education'),
    ('property_management', 'property-management'),
    ('pub', 'bars-breweries'),
    ('ramen_restaurant', 'japanese'),
    ('real_estate_agency', 'real-estate-agent'),
    ('realtor', 'real-estate-agent'),
    ('restaurant', 'food-drink'),
    ('roofer', 'roofing'),
    ('roofing_contractor', 'roofing'),
    ('salad_shop', 'food-drink'),
    ('sandwich_shop', 'food-drink'),
    ('school', 'child-care-education'),
    ('screen_enclosure', 'screen-enclosures'),
    ('seafood_restaurant', 'seafood'),
    ('secondary_school', 'child-care-education'),
    ('shoe_store', 'boutiques-thrift'),
    ('skin_care_clinic', 'medical-spa'),
    ('soup_shop', 'food-drink'),
    ('southern_restaurant', 'american'),
    ('spa', 'day-spa'),
    ('spanish_restaurant', 'food-drink'),
    ('specialty_grocery', 'specialty-grocery-gourmet'),
    ('sporting_goods_store', 'books-music-hobby'),
    ('sports_bar', 'bars-breweries'),
    ('sports_coaching', 'fitness-studio'),
    ('stationery_store', 'gifts-stationery'),
    ('steak_house', 'steakhouse'),
    ('steakhouse', 'steakhouse'),
    ('supermarket', 'specialty-grocery-gourmet'),
    ('sushi_restaurant', 'japanese'),
    ('taco_restaurant', 'mexican'),
    ('tattoo_parlor', 'tattoo-parlor'),
    ('tax_preparation', 'accountant'),
    ('tea_house', 'coffee-shop'),
    ('tea_store', 'coffee-shop'),
    ('tex_mex_restaurant', 'mexican'),
    ('thai_restaurant', 'food-drink'),
    ('thrift_store', 'boutiques-thrift'),
    ('tire_shop', 'tire-oil-change'),
    ('tour_agency', 'local-attractions-tours'),
    ('tourist_attraction', 'local-attractions-tours'),
    ('toy_store', 'books-music-hobby'),
    ('travel_agency', 'local-attractions-tours'),
    ('tree_service', 'tree-service'),
    ('turkish_restaurant', 'food-drink'),
    ('tutoring_service', 'child-care-education'),
    ('vegan_restaurant', 'food-drink'),
    ('vegetarian_restaurant', 'food-drink'),
    ('veterinary_care', 'veterinarian'),
    ('vietnamese_restaurant', 'food-drink'),
    ('vintage_clothing_store', 'boutiques-thrift'),
    ('wedding_venue', 'special-events'),
    ('wellness_center', 'health-wellness'),
    ('wine_bar', 'bars-breweries'),
    ('yoga_studio', 'fitness-studio')
ON CONFLICT (google_type) DO NOTHING;

-- The guard looks a listing up in the Places cache by place_id, else by name.
CREATE INDEX IF NOT EXISTS google_places_cache_name_lower_idx
    ON google_places_cache (lower(name));

CREATE OR REPLACE FUNCTION businesses_require_category() RETURNS trigger AS $$
BEGIN
    IF NEW.is_active IS TRUE AND NEW.category_id IS NULL THEN
        SELECT c.id INTO NEW.category_id
        FROM google_places_cache g
        CROSS JOIN LATERAL unnest(g.types) WITH ORDINALITY AS t(google_type, ord)
        JOIN business_type_category_map m ON m.google_type = t.google_type
        JOIN directory_categories c
             ON c.slug = m.category_slug
            AND (c.directory_id IS NULL OR c.directory_id = NEW.directory_id)
        WHERE (NEW.place_id IS NOT NULL AND g.place_id = NEW.place_id)
           OR lower(g.name) = lower(NEW.name)
        ORDER BY (NEW.place_id IS NOT NULL AND g.place_id = NEW.place_id) DESC,
                 (c.directory_id = NEW.directory_id) DESC,
                 t.ord
        LIMIT 1;

        -- Nothing in the source data names a trade: keep the row, refuse to publish it.
        -- A category-less page is worse than an unpublished listing.
        IF NEW.category_id IS NULL THEN
            NEW.is_active := false;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS businesses_require_category_trg ON businesses;
CREATE TRIGGER businesses_require_category_trg
    BEFORE INSERT OR UPDATE OF is_active, category_id, name, place_id ON businesses
    FOR EACH ROW EXECUTE FUNCTION businesses_require_category();

COMMENT ON TABLE business_type_category_map IS
    'Google Places type -> directory category slug. Single vocabulary for the '
    'no-active-listing-without-a-category guard (card t_e092976e).';
