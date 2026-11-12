//! Category-vocabulary guard (kanban t_6b831daa).
//!
//! Measured live 2026-11-12: businesses were filed under a trade their own name contradicts —
//! "Fine Ink Studios" (a tattoo studio) under *Fitness Studio*, a dozen CPA firms under
//! *Personal Injury Lawyer*, "HOTWORX" (a gym) under *Landscaping*. The importer's
//! `map_category` fell back to the SEARCH TERM's category for any candidate whose Google types
//! matched nothing, so a broad Places result stamped the wrong trade on the listing.
//!
//! This module holds the one vocabulary both the importer and any backfill check against:
//! what a business NAME (and its Google types) implies, and which family a category belongs to.
//! A category in a different family than the name implies is a CONTRADICTION and must never be
//! assigned. Generic buckets ("Other", "Professional", "Health & Wellness") never contradict —
//! they are honest labels for an uncertain trade.

use uuid::Uuid;

/// Family -> name tokens. A NAME containing one of these belongs to that family.
/// Keep in sync with the families used by `category_family` below.
pub const FAMILY_TOKENS: &[(&str, &[&str])] = &[
    (
        "tattoo",
        &["tattoo", "ink studio", "ink studios", "ink lab"],
    ),
    (
        "fitness",
        &[
            "gym",
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
            "kickboxing",
        ],
    ),
    (
        "landscaping",
        &[
            "landscaping",
            "landscape",
            "lawn care",
            "lawncare",
            "lawn service",
            "lawn",
        ],
    ),
    (
        "dentist",
        &["dental", "dentist", "orthodont", "endodont", "periodont"],
    ),
    ("plumber", &["plumbing", "plumber", "rooter"]),
    ("roofing", &["roofing", "roofer"]),
    (
        "hvac",
        &[
            "hvac",
            "air conditioning",
            "heating and air",
            "heating & air",
            "air & heat",
            "cooling & heating",
        ],
    ),
    (
        "hair",
        &[
            "hair",
            "barber",
            "great clips",
            "supercuts",
            "sport clips",
            "blowout",
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
        ],
    ),
    ("spa", &["day spa", "spa &", "spa and", "& spa", " spa"]),
    ("chiro", &["chiro"]),
    ("physical_therapy", &["physical therapy", "physiotherapy"]),
    (
        "vet",
        &[
            "veterinary",
            "vet clinic",
            "animal hospital",
            "pet hospital",
            "animal clinic",
            "animal medical",
        ],
    ),
    (
        "groomer",
        &["groomer", "grooming", "pet groom", "dog groom"],
    ),
    ("pizza", &["pizza", "pizzeria"]),
    ("bakery", &["bakery", "bake shop", "patisserie", "pastry"]),
    (
        "coffee",
        &[
            "coffee",
            "espresso",
            "roasters",
            "cafe",
            "café",
            "tea house",
        ],
    ),
    (
        "accountant",
        &[
            "cpa",
            "accounting",
            "tax service",
            "bookkeeping",
            "tax &",
            "tax and",
        ],
    ),
    (
        "lawyer",
        &[
            "law office",
            "law firm",
            "lawyer",
            "attorney",
            "injury",
            "legal",
        ],
    ),
    ("realestate", &["realty", "real estate", "realtor"]),
    ("property_mgmt", &["property management"]),
    (
        "pool",
        &[
            "pool service",
            "pool cleaning",
            "pool care",
            "pools",
            "pool cleaners",
            "pool ",
        ],
    ),
    (
        "tree",
        &[
            "tree service",
            "tree care",
            "arborist",
            "tree removal",
            "tree trimming",
        ],
    ),
    (
        "cleaning",
        &[
            "cleaning",
            "cleaners",
            "janitorial",
            "maid service",
            "maids",
            "pressure washing",
        ],
    ),
    ("electrician", &["electric", "electrical"]),
    (
        "auto",
        &[
            "auto repair",
            "automotive",
            "mechanic",
            "tire",
            "oil change",
            "transmission",
            "brake",
            "collision",
            "body shop",
            "car wash",
            "detailing",
        ],
    ),
    ("boat", &["boat", "charter", "sailing", "marina"]),
    (
        "childcare",
        &[
            "daycare",
            "day care",
            "preschool",
            "montessori",
            "learning center",
            "kids academy",
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
];

/// Google Places type -> family. Used when the NAME says nothing.
pub const TYPE_FAMILIES: &[(&str, &str)] = &[
    ("gym", "fitness"),
    ("fitness_center", "fitness"),
    ("tattoo_parlor", "tattoo"),
    ("hair_care", "hair"),
    ("hair_salon", "hair"),
    ("beauty_salon", "hair"),
    ("spa", "spa"),
    ("dentist", "dentist"),
    ("plumber", "plumber"),
    ("roofing_contractor", "roofing"),
    ("electrician", "electrician"),
    ("accounting", "accountant"),
    ("lawyer", "lawyer"),
    ("physiotherapist", "physical_therapy"),
    ("chiropractor", "chiro"),
    ("veterinary_care", "vet"),
    ("pet_groomer", "groomer"),
    ("restaurant", "food"),
    ("cafe", "food"),
    ("bakery", "food"),
    ("meal_takeaway", "food"),
    ("bar", "food"),
    ("real_estate_agency", "realestate"),
    ("car_repair", "auto"),
    ("car_wash", "auto"),
    ("car_dealer", "auto"),
    ("laundry", "cleaning"),
    ("school", "childcare"),
    ("child_care_agency", "childcare"),
];

fn cluster(family: &str) -> &str {
    CLUSTERS
        .iter()
        .find(|(f, _)| *f == family)
        .map(|(_, c)| *c)
        .unwrap_or(family)
}

/// Which trade families this business looks like, from its own name and Google types.
pub fn implied_families(name: &str, types: &[String]) -> Vec<&'static str> {
    let n = format!(" {} ", name.to_lowercase());
    let mut out: Vec<&'static str> = Vec::new();
    for (family, tokens) in FAMILY_TOKENS {
        if tokens.iter().any(|t| n.contains(t)) {
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

/// True when the category names a trade the business's own name/types contradict.
pub fn contradicts(name: &str, types: &[String], category_name: &str) -> bool {
    let implied = implied_families(name, types);
    if implied.is_empty() {
        return false;
    }
    let Some(fam) = category_family(category_name) else {
        return false; // generic bucket — never a contradiction
    };
    let target = cluster(fam);
    !implied.iter().any(|i| cluster(i) == target)
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
}
