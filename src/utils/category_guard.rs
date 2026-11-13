//! Category-vocabulary guard (kanban t_6b831daa, extended by t_b2f7201e).
//!
//! Measured live 2026-11-12: businesses were filed under a trade their own name contradicts —
//! "Fine Ink Studios" (a tattoo studio) under *Fitness Studio*, a dozen CPA firms under
//! *Personal Injury Lawyer*, "HOTWORX" (a gym) under *Landscaping*. The importer's
//! `map_category` fell back to the SEARCH TERM's category for any candidate whose Google types
//! matched nothing, so a broad Places result stamped the wrong trade on the listing.
//!
//! Measured again 2026-11-13 (t_b2f7201e): the guard only vetoed a category the NAME contradicts,
//! so every listing whose name is a brand and names no trade at all passed every check — "Blo Blow
//! Dry Bar", "FIT 407", "Vitalounge" and "The Hangry Bison" were all published as *Landscaping*.
//! Two things were missing: the provider's own types were never consulted for the rows that had
//! them, and one narrow vocabulary made a real contradiction look like silence.
//!
//! The rule is now a single three-way VERDICT shared by the importer and the backfill:
//!   Supported   — the name or a specific Places type says the assigned trade;
//!   Contradicted — the name/types name a different trade and nothing defends the assigned one;
//!   Unknown      — no evidence, or conflicting evidence. An honest blank, never a guess.
//! Only `Contradicted` may ever move a listing, and only to a category the evidence names.

use uuid::Uuid;

/// Family -> name tokens. A NAME containing one of these as a whole word belongs to that family.
/// Keep in sync with `category_family` below.
pub const FAMILY_TOKENS: &[(&str, &[&str])] = &[
    (
        "tattoo",
        &[
            "tattoo",
            "tattoos",
            "tattooing",
            "piercing",
            "piercings",
            "ink",
            "ink studio",
            "ink studios",
            "ink lab",
        ],
    ),
    (
        "fitness",
        &[
            "gym",
            "gyms",
            "fitness",
            "hotworx",
            "crossfit",
            "pilates",
            "yoga",
            "orangetheory",
            "anytime fitness",
            "planet fitness",
            "la fitness",
            "esporta",
            "crunch fitness",
            "burn boot camp",
            "f45",
            "9round",
            "title boxing",
            "jiu jitsu",
            "jiu-jitsu",
            "martial arts",
            "martial",
            "kickboxing",
            "boxing",
            "athlete",
            "athletic",
            "athletics",
            "workout",
            "jazzercise",
            "barre",
        ],
    ),
    (
        "landscaping",
        &[
            "landscaping",
            "landscape",
            "landscapes",
            "landscaper",
            "lawn care",
            "lawncare",
            "lawn service",
            "lawn",
            "lawns",
            "mowing",
            "irrigation",
            "sprinkler",
            "sprinklers",
            "turf",
            "horticulture",
            "arbor",
            "grounds",
            "sod",
        ],
    ),
    (
        "dentist",
        &[
            "dental",
            "dentist",
            "dentists",
            "dentistry",
            "orthodontics",
            "orthodontic",
            "endodontics",
            "periodontics",
        ],
    ),
    ("plumber", &["plumbing", "plumber", "plumbers", "rooter"]),
    (
        "roofing",
        &["roofing", "roofer", "roofers", "roof", "roofs"],
    ),
    (
        "hvac",
        &[
            "hvac",
            "heating",
            "cooling",
            "furnace",
            "refrigeration",
            "air conditioning",
            "heating and air",
        ],
    ),
    (
        "hair",
        &[
            "hair",
            "barber",
            "barbers",
            "barbershop",
            "salon",
            "salons",
            "great clips",
            "supercuts",
            "sport clips",
            "blowout",
            "blowouts",
            "blowdry",
            "styling",
        ],
    ),
    (
        "medical_spa",
        &[
            "med spa",
            "medical spa",
            "medspa",
            "aesthetics",
            "injectables",
            "botox",
            "hydrafacial",
        ],
    ),
    ("spa", &["spa", "spas", "massage", "facial", "facials"]),
    (
        "chiro",
        &["chiro", "chiropractic", "chiropractor", "chiropractors"],
    ),
    (
        "physical_therapy",
        &[
            "physical therapy",
            "physiotherapy",
            "physiotherapist",
            "rehab",
            "rehabilitation",
        ],
    ),
    (
        "vet",
        &[
            "veterinary",
            "veterinarian",
            "veterinarians",
            "vet",
            "vets",
            "animal hospital",
            "pet hospital",
            "animal clinic",
            "animal medical",
        ],
    ),
    (
        "groomer",
        &[
            "groomer",
            "groomers",
            "grooming",
            "groom",
            "pet groom",
            "dog groom",
        ],
    ),
    ("pizza", &["pizza", "pizzas", "pizzeria", "pizzerias"]),
    (
        "bakery",
        &[
            "bakery",
            "bakeries",
            "bake",
            "bakeshop",
            "patisserie",
            "pastry",
            "pastries",
            "macarons",
            "donuts",
            "donut",
            "doughnuts",
            "cupcakes",
            "cupcake",
            "dessert",
            "desserts",
        ],
    ),
    (
        "coffee",
        &[
            "coffee",
            "espresso",
            "roasters",
            "roaster",
            "cafe",
            "cafes",
            "café",
            "tea house",
            "teahouse",
        ],
    ),
    (
        "accountant",
        &[
            "cpa",
            "accounting",
            "accountants",
            "accountant",
            "tax",
            "taxes",
            "bookkeeping",
            "bookkeeper",
            "payroll",
        ],
    ),
    (
        "lawyer",
        &[
            "law office",
            "law firm",
            "lawyer",
            "lawyers",
            "attorney",
            "attorneys",
            "law",
            "laws",
            "injury",
            "injuries",
            "legal",
        ],
    ),
    (
        "realestate",
        &["realty", "realtor", "realtors", "real estate"],
    ),
    (
        "property_mgmt",
        &["property management", "property", "properties", "hoa"],
    ),
    (
        "pool",
        &[
            "pool service",
            "pool cleaning",
            "pool care",
            "poolcare",
            "pools",
            "pool",
            "pool cleaners",
        ],
    ),
    (
        "tree",
        &[
            "tree service",
            "tree care",
            "arborist",
            "arborcare",
            "tree removal",
            "tree trimming",
            "tree",
            "trees",
            "stump",
            "stumps",
        ],
    ),
    (
        "cleaning",
        &[
            "cleaning",
            "cleaners",
            "cleaner",
            "janitorial",
            "maid service",
            "maid",
            "maids",
            "housekeeping",
            "pressure washing",
        ],
    ),
    (
        "electrician",
        &["electric", "electrical", "electrician", "electricians"],
    ),
    (
        "auto",
        &[
            "auto repair",
            "automotive",
            "mechanic",
            "mechanics",
            "tire",
            "tires",
            "towing",
            "oil change",
            "transmission",
            "brakes",
            "brake",
            "collision",
            "body shop",
            "car wash",
            "detailing",
            "muffler",
        ],
    ),
    (
        "boat",
        &[
            "boat", "boats", "charter", "charters", "sailing", "marina", "pontoon", "pontoons",
            "yacht", "yachts",
        ],
    ),
    (
        "childcare",
        &[
            "daycare",
            "day care",
            "preschool",
            "montessori",
            "learning center",
            "kids academy",
            "childcare",
        ],
    ),
    (
        "food",
        &[
            "restaurant",
            "restaurants",
            "taqueria",
            "grill",
            "bistro",
            "diner",
            "eatery",
            "cantina",
            "brewery",
            "brewing",
            "tavern",
            "pub",
            "steakhouse",
            "sushi",
        ],
    ),
    (
        "contractor",
        &[
            "handyman",
            "renovations",
            "remodeling",
            "construction",
            "contracting",
            "enclosures",
        ],
    ),
    (
        "medical",
        &[
            "medical",
            "clinic",
            "healthcare",
            "ophthalmology",
            "optometry",
            "optometrist",
            "hospital",
            "surgery",
            "surgeon",
            "physician",
        ],
    ),
];

/// Category name (lowercase) -> family. Only CONCRETE categories are listed; anything absent is
/// treated as a generic bucket and can never contradict.
pub const CATEGORY_FAMILIES: &[(&str, &str)] = &[
    ("tattoo parlor", "tattoo"),
    ("fitness studio", "fitness"),
    ("landscaping", "landscaping"),
    ("dentist", "dentist"),
    ("plumber", "plumber"),
    ("roofing", "roofing"),
    ("hvac", "hvac"),
    ("hair salon", "hair"),
    ("day spa", "spa"),
    ("medical spa", "medical_spa"),
    ("chiropractor", "chiro"),
    ("physical therapy", "physical_therapy"),
    ("veterinarian", "vet"),
    ("pet groomer", "groomer"),
    ("pizza", "pizza"),
    ("bakeries & dessert shops", "bakery"),
    ("coffee shop", "coffee"),
    ("accountant", "accountant"),
    ("personal injury lawyer", "lawyer"),
    ("real estate agent", "realestate"),
    ("property management", "property_mgmt"),
    ("pool service", "pool"),
    ("tree service", "tree"),
    ("cleaning services", "cleaning"),
    ("electrician", "electrician"),
    ("auto repair", "auto"),
    ("tire shops & oil change", "auto"),
    ("car detailing & wash", "auto"),
    ("boat charter", "boat"),
    ("child care & education", "childcare"),
    ("dog daycare & boarding", "childcare"),
    ("maintenance & handyman", "contractor"),
    ("screen enclosures", "contractor"),
    ("medical & health services", "medical"),
    // Every cuisine and drink category is a food business: a restaurant type supports all of them,
    // and a restaurant type under *Landscaping* is exactly the defect this guard must catch.
    ("american", "food"),
    ("italian", "food"),
    ("mexican", "food"),
    ("chinese", "food"),
    ("japanese", "food"),
    ("indian", "food"),
    ("french", "food"),
    ("mediterranean", "food"),
    ("seafood", "food"),
    ("steakhouse", "food"),
    ("breakfast", "food"),
    ("fine dining", "food"),
    ("food & drink", "food"),
    ("food trucks", "food"),
    ("catering", "food"),
    ("bars & breweries", "food"),
    ("live music & bars", "food"),
];

/// Families that legitimately describe the same business, so a listing filed in one while its
/// name mentions the other is NOT a contradiction.
pub const CLUSTERS: &[(&str, &str)] = &[
    ("hair", "beauty"),
    ("spa", "beauty"),
    ("medical_spa", "beauty"),
    ("chiro", "therapy"),
    ("physical_therapy", "therapy"),
    ("vet", "animal"),
    ("groomer", "animal"),
    ("bakery", "food"),
    ("coffee", "food"),
    ("pizza", "food"),
    // A tree surgeon and a landscape contractor are the same trade in practice; so is a
    // property manager and a real-estate agency. In this market "air conditioning & plumbing" is
    // one company more often than not, so neither label contradicts the other.
    ("landscaping", "grounds"),
    ("tree", "grounds"),
    ("realestate", "realty"),
    ("property_mgmt", "realty"),
    ("hvac", "mechanical"),
    ("plumber", "mechanical"),
];

/// Google Places type -> family. Only SPECIFIC provider types appear here. Broad types
/// (`general_contractor`, `school`, `food`, `point_of_interest`, `store`, `health`,
/// `medical_clinic`) are deliberately absent: measured 2026-11-13 they are attached to
/// landscapers, senior-living communities and water parks, and treating them as evidence
/// re-files correct listings.
pub const TYPE_FAMILIES: &[(&str, &str)] = &[
    ("gym", "fitness"),
    ("fitness_center", "fitness"),
    ("tattoo_parlor", "tattoo"),
    ("body_art_service", "tattoo"),
    ("hair_care", "hair"),
    ("hair_salon", "hair"),
    ("beauty_salon", "hair"),
    ("spa", "spa"),
    ("beauty", "spa"),
    ("massage", "spa"),
    ("swimming_pool", "pool"),
    ("dentist", "dentist"),
    ("dental_clinic", "dentist"),
    ("plumber", "plumber"),
    ("roofing_contractor", "roofing"),
    ("electrician", "electrician"),
    ("accounting", "accountant"),
    ("lawyer", "lawyer"),
    ("physiotherapist", "physical_therapy"),
    ("chiropractor", "chiro"),
    ("veterinary_care", "vet"),
    ("pet_groomer", "groomer"),
    ("pet_care", "groomer"),
    ("restaurant", "food"),
    ("cafe", "food"),
    ("bakery", "food"),
    ("meal_takeaway", "food"),
    ("meal_delivery", "food"),
    ("bar", "food"),
    ("pub", "food"),
    ("night_club", "food"),
    ("real_estate_agency", "realestate"),
    ("car_repair", "auto"),
    ("car_wash", "auto"),
    ("car_dealer", "auto"),
    ("laundry", "cleaning"),
    ("child_care_agency", "childcare"),
    ("preschool", "childcare"),
];

/// The category a family may be moved TO when the evidence contradicts the current one.
/// Only honest, existing buckets: a cuisine is never guessed from a bare `restaurant` type,
/// and a bare `lawyer` type never claims personal-injury work.
pub const FAMILY_TARGETS: &[(&str, &str)] = &[
    ("vet", "Veterinarian"),
    ("medical_spa", "Medical Spa"),
    ("spa", "Day Spa"),
    ("hair", "Hair Salon"),
    ("groomer", "Pet Groomer"),
    ("tattoo", "Tattoo Parlor"),
    ("dentist", "Dentist"),
    ("plumber", "Plumber"),
    ("roofing", "Roofing"),
    ("hvac", "HVAC"),
    ("electrician", "Electrician"),
    ("accountant", "Accountant"),
    ("lawyer", "Professional"),
    ("chiro", "Chiropractor"),
    ("physical_therapy", "Physical Therapy"),
    ("landscaping", "Landscaping"),
    ("tree", "Tree Service"),
    ("pool", "Pool Service"),
    ("fitness", "Fitness Studio"),
    ("auto", "Auto Repair"),
    ("boat", "Boat Charter"),
    ("cleaning", "Cleaning Services"),
    ("childcare", "Child Care & Education"),
    ("food", "Food & Drink"),
    ("pizza", "Pizza"),
    ("bakery", "Bakeries & Dessert Shops"),
    ("coffee", "Coffee Shop"),
    ("realestate", "Real Estate Agent"),
    ("property_mgmt", "Property Management"),
    ("medical", "Medical & Health Services"),
    ("contractor", "Maintenance & Handyman"),
];

/// Which family wins when several point at the same business. Most specific first.
const TARGET_PREFERENCE: &[&str] = &[
    "vet",
    "medical_spa",
    "spa",
    "hair",
    "groomer",
    "tattoo",
    "dentist",
    "plumber",
    "roofing",
    "hvac",
    "electrician",
    "accountant",
    "lawyer",
    "chiro",
    "physical_therapy",
    "landscaping",
    "tree",
    "pool",
    "fitness",
    "auto",
    "boat",
    "cleaning",
    "childcare",
    "food",
    "pizza",
    "bakery",
    "coffee",
    "realestate",
    "property_mgmt",
    "medical",
    "contractor",
];

/// What the evidence says about one (name, types, category) triple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The assigned category is one the evidence names.
    Supported,
    /// The evidence names another trade and nothing defends the assigned category. Carries the
    /// families the evidence points at, most specific last, so a caller can pick a target.
    Contradicted(Vec<&'static str>),
    /// No evidence, or evidence pointing two ways. Never acted on.
    Unknown,
}

fn cluster(family: &str) -> &str {
    CLUSTERS
        .iter()
        .find(|(f, _)| *f == family)
        .map(|(_, c)| *c)
        .unwrap_or(family)
}

/// Lowercase the name and reduce every non-alphanumeric run to one space, then pad with spaces,
/// so a token can be matched as whole words. Substring matching ("pool " inside "Pooler") had no
/// way to say "this word, exactly" — and "fit" inside "outfit" is how a vocabulary starts lying.
fn normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push(' ');
    let mut space = true;
    for ch in name.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            out.push(ch);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    if !space {
        out.push(' ');
    }
    out
}

fn has_token(normalised: &str, token: &str) -> bool {
    let mut needle = String::with_capacity(token.len() + 2);
    needle.push(' ');
    needle.push_str(token);
    needle.push(' ');
    normalised.contains(&needle)
}

/// Which trade families this business looks like, from its own name and Google types.
pub fn implied_families(name: &str, types: &[String]) -> Vec<&'static str> {
    let n = normalise(name);
    let mut out: Vec<&'static str> = Vec::new();
    for (family, tokens) in FAMILY_TOKENS {
        if tokens.iter().any(|t| has_token(&n, t)) && !out.contains(family) {
            out.push(family);
        }
    }
    for t in types {
        let tl = t.to_lowercase();
        if let Some((_, family)) = TYPE_FAMILIES.iter().find(|(gt, _)| *gt == tl) {
            if !out.contains(family) {
                out.push(family);
            }
        }
    }
    // "Pool Cleaners" is a pool business, not a general cleaner.
    if out.contains(&"pool") {
        out.retain(|f| *f != "cleaning");
    }
    out
}

/// The concrete family a category belongs to, if it names one.
pub fn category_family(category_name: &str) -> Option<&'static str> {
    let key = category_name.trim().to_lowercase();
    CATEGORY_FAMILIES
        .iter()
        .find(|(cat, _)| *cat == key)
        .map(|(_, f)| *f)
}

/// Words of a category name that can defend a listing when they appear in its OWN name —
/// `Select Physical Therapy` is a physical-therapy clinic whatever Places tags it.
///
/// Generic heads are excluded on purpose: "studio" sits in Fitness Studio, in every hair studio
/// and in Tattoo Parlor's neighbours, so letting it defend a category would make "High Class Ink
/// Studio" a supported Fitness Studio. Only a word that names the trade counts.
fn category_words(category_name: &str) -> Vec<String> {
    const GENERIC: &[&str] = &[
        "shops", "shop", "services", "service", "other", "studio", "studios", "center", "centre",
        "care", "day", "the", "and", "repair",
    ];
    category_name
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !GENERIC.contains(w))
        .map(str::to_string)
        .collect()
}

/// The single rule. Three-way, so "no evidence" can never be mistaken for "contradiction".
pub fn verdict(name: &str, types: &[String], category_name: &str) -> Verdict {
    let Some(fam) = category_family(category_name) else {
        return Verdict::Unknown; // generic bucket — never a contradiction
    };
    let n = normalise(name);
    if category_words(category_name)
        .iter()
        .any(|w| has_token(&n, w))
    {
        return Verdict::Supported;
    }
    let implied = implied_families(name, types);
    if implied.iter().any(|i| cluster(i) == cluster(fam)) {
        return Verdict::Supported;
    }
    if implied.is_empty() {
        return Verdict::Unknown;
    }
    // Conflicting evidence is not proof: a name that says "salon" and a type that says "vet"
    // leaves us unable to say which trade is right.
    let mut clusters: Vec<&str> = implied.iter().map(|f| cluster(f)).collect();
    clusters.sort_unstable();
    clusters.dedup();
    if clusters.len() > 1 {
        return Verdict::Unknown;
    }
    Verdict::Contradicted(implied)
}

/// True when the category names a trade the business's own name/types contradict.
pub fn contradicts(name: &str, types: &[String], category_name: &str) -> bool {
    matches!(
        verdict(name, types, category_name),
        Verdict::Contradicted(_)
    )
}

/// The category a contradicted listing should move to, if the evidence names one we hold.
pub fn target_category(families: &[&str]) -> Option<&'static str> {
    for want in TARGET_PREFERENCE {
        if families.contains(want) {
            if let Some((_, cat)) = FAMILY_TARGETS.iter().find(|(f, _)| f == want) {
                return Some(cat);
            }
        }
    }
    None
}

/// The category in `cats` that the name/types actually point at, if one exists.
pub fn best_category(name: &str, types: &[String], cats: &[(Uuid, String)]) -> Option<usize> {
    let implied = implied_families(name, types);
    if implied.is_empty() {
        return None;
    }
    for want in &implied {
        let target = cluster(want);
        if let Some((idx, _)) = cats.iter().enumerate().find(|(_, (_, cn))| {
            category_family(cn)
                .map(|f| cluster(f) == target)
                .unwrap_or(false)
        }) {
            return Some(idx);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tattoo_studio_under_fitness_studio_contradicts() {
        assert!(contradicts("Fine Ink Studios", &[], "Fitness Studio"));
        assert!(contradicts("High Class Ink Studio", &[], "Fitness Studio"));
    }

    #[test]
    fn cpa_firm_under_personal_injury_lawyer_contradicts() {
        assert!(contradicts(
            "N. Persaud, CPA & Company, LLC",
            &[],
            "Personal Injury Lawyer"
        ));
    }

    #[test]
    fn gym_under_landscaping_contradicts() {
        assert!(contradicts(
            "HOTWORX - Winter Garden, FL",
            &types(&["gym"]),
            "Landscaping"
        ));
    }

    #[test]
    fn a_brand_name_with_no_trade_is_judged_by_places_types() {
        // The t_b2f7201e defect: these names name no trade, so the old guard passed them and the
        // importer stamped the search term's category on them.
        assert!(contradicts(
            "Blo Blow Dry Bar Winter Garden",
            &types(&["hair_salon", "hair_care"]),
            "Landscaping"
        ));
        assert!(contradicts(
            "FIT 407 Winter Garden",
            &types(&["gym", "sports_complex"]),
            "Landscaping"
        ));
        assert!(contradicts(
            "Vitalounge Winter Garden",
            &types(&["spa", "wellness_center"]),
            "Landscaping"
        ));
        assert!(contradicts(
            "The Hangry Bison - Winter Garden",
            &types(&["american_restaurant", "restaurant"]),
            "Landscaping"
        ));
        // …and with no types at all it is still an honest blank, not a guess.
        assert!(!contradicts("Vitalounge Winter Garden", &[], "Landscaping"));
    }

    #[test]
    fn broad_provider_types_are_not_evidence() {
        // Measured 2026-11-13: every one of these carries `general_contractor`/`school`/`food`
        // and was correctly filed. Treating those types as evidence re-files correct listings.
        assert!(!contradicts(
            "Ken Williams Air Conditioning",
            &types(&["general_contractor", "point_of_interest", "service"]),
            "HVAC"
        ));
        assert!(!contradicts(
            "Buena Vista Watersports",
            &types(&["tourist_attraction", "school", "sports_coaching"]),
            "Fitness Studio"
        ));
        // a stretch studio is tagged wellness_center, a kayak rental sports_activity_location:
        // neither says "this is a day spa" or "this is a gym"
        assert!(!contradicts(
            "Stretch Zone",
            &types(&["wellness_center", "health"]),
            "Fitness Studio"
        ));
        assert!(!contradicts(
            "Get Up And Go Kayaking - Rock Springs",
            &types(&["tourist_attraction", "sports_activity_location"]),
            "Boat Charter"
        ));
    }

    #[test]
    fn heating_and_plumbing_are_one_mechanical_trade() {
        assert!(!contradicts(
            "Rule The Air",
            &types(&["plumber", "general_contractor"]),
            "HVAC"
        ));
    }

    #[test]
    fn correct_and_ambiguous_filings_do_not_contradict() {
        assert!(!contradicts("Fine Ink Studios", &[], "Tattoo Parlor"));
        // salon + spa is one business, whichever of the three beauty categories it is filed in
        assert!(!contradicts("Tomy Mishali Salon & Spa", &[], "Hair Salon"));
        assert!(!contradicts("Be'u Salon & Spa", &[], "Day Spa"));
        // a pool cleaner is not a "cleaner" contradiction
        assert!(!contradicts(
            "Total Weekly Pool Cleaners",
            &[],
            "Pool Service"
        ));
        // generic buckets never contradict
        assert!(!contradicts("HOTWORX - Winter Garden, FL", &[], "Other"));
        assert!(!contradicts("Fine Ink Studios", &[], "Health & Wellness"));
        // nothing implied -> no verdict
        assert!(!contradicts("Sunrise Plaza LLC", &[], "Fitness Studio"));
        // a name that says the trade defends it, whatever the provider types claim
        assert!(!contradicts(
            "Select Physical Therapy - East Boca",
            &types(&["spa"]),
            "Physical Therapy"
        ));
        // a name that says a DIFFERENT trade is conflicting evidence, not proof
        assert!(!contradicts(
            "Lake Nona Ophthalmology",
            &types(&["hair_care", "spa"]),
            "Chiropractor"
        ));
    }

    #[test]
    fn adjacent_trades_share_a_cluster() {
        assert!(!contradicts(
            "East Coast Arbor Pro Inc.",
            &[],
            "Tree Service"
        ));
        assert!(!contradicts(
            "County Line Ornamental & Tree",
            &[],
            "Landscaping"
        ));
        assert!(!contradicts(
            "Vesta Property Services",
            &[],
            "Property Management"
        ));
    }

    #[test]
    fn whole_word_tokens_do_not_fire_inside_other_words() {
        // "fit" is not a token, so "Outfit" is not a gym; "spa" must not match "Sparrow".
        assert_eq!(
            implied_families("Outfit Collective", &[]),
            Vec::<&str>::new()
        );
        assert_eq!(implied_families("Sparrow & Co", &[]), Vec::<&str>::new());
    }

    #[test]
    fn best_category_upgrades_a_misfiled_import() {
        let cats = vec![
            (Uuid::nil(), "Personal Injury Lawyer".to_string()),
            (Uuid::nil(), "Accountant".to_string()),
        ];
        let idx = best_category("Pelican Tax CPA", &[], &cats).unwrap();
        assert_eq!(cats[idx].1, "Accountant");
    }

    #[test]
    fn google_type_can_supply_the_family() {
        assert_eq!(
            implied_families("HOTWORX - Winter Garden", &types(&["gym"])),
            vec!["fitness"]
        );
    }

    #[test]
    fn contradicted_family_picks_a_specific_target() {
        assert_eq!(target_category(&["groomer", "vet"]), Some("Veterinarian"));
        assert_eq!(target_category(&["hair"]), Some("Hair Salon"));
        assert_eq!(target_category(&["food"]), Some("Food & Drink"));
    }
}
