//! Franchise / big-chain detection.
//!
//! The public community directory is for small/local businesses. National
//! franchises and big chains should be flagged (`is_franchise = true`) so they
//! are excluded from public listings. Detection is a *hint*, not a hard rule —
//! the admin can always override before publishing.
//!
//! Signals used:
//!   1. Known-chain keyword match against the business name.
//!   2. Google Places `types` containing chain indicators
//!      (`restaurant`, `store`, `supermarket`, etc. combined with known brands,
//!      or `lodging` / `airport` / `bank` which are rarely local-only).

/// Known national franchise / big-chain brand names (case-insensitive substring match).
/// Additive list — extend as needed, never remove reliability for existing names.
pub const KNOWN_CHAINS: &[&str] = &[
    "mcdonald", "burger king", "wendy", "subway", "starbucks", "dunkin", "taco bell",
    "kfc", "pizza hut", "domino", "chipotle", "panera", "chick-fil-a", "five guys",
    "papa john", "little caesar", "arbys", "sonic drive", "dairy queen", "popeyes",
    "carl's jr", "hardee", "jack in the box", "whataburger", "in-n-out", "shakeshack",
    "olive garden", "red lobster", "outback steakhouse", "cheesecake factory",
    "applebees", "chili", "tgi fridays", "buffalo wild wings", "wingstop",
    "denny", "ihop", "waffle house", "cracker barrel", "bob evans", "golden corral",
    "texas roadhouse", "longhorn steakhouse", "cava", "sweetgreen", "jersey mike",
    "firehouse subs", "jimmy john", "potbelly", "moe's southwest", "qdob",
    "walmart", "target", "costco", "sams club", "best buy", "home depot", "lowes",
    "kroger", "publix", "whole foods", "trader joe", "aldi", "dollar general",
    "dollar tree", "family dollar", "walgreens", "cvs", "rite aid", "7-eleven",
    "circle k", "speedway", "shell", "chevron", "bp", "exxon", "mobil",
    "marriott", "hilton", "hyatt", "holiday inn", "best western", "motel 6",
    "la quinta", "days inn", "comfort inn", "super 8", "ramada", "quality inn",
    "ups store", "fedex office", "fedex kinkos", "geico", "state farm", "allstate",
    "progressive", "h&r block", "jackson hewitt", "liberty tax", "planet fitness",
    "anytime fitness", "orangetheory", "crunch fitness", "gold's gym", "24 hour fitness",
    "great clips", "supercuts", "sport clips", "jiffy lube", "valvoline", "firestone",
    "pep boys", "ntb", "meineke", "midas", "aamco", "maaco", "goodyear",
];

/// Google Places `types` that indicate a big-chain / non-local establishment.
pub const CHAIN_TYPES: &[&str] = &[
    "airport", "bank", "gas_station", "supermarket", "department_store",
    "shopping_mall", "car_rental", "car_dealer", "lodging", "casino",
    "movie_theater", "pharmacy", "convenience_store", "furniture_store",
    "electronics_store", "hardware_store", "home_goods_store", "shoe_store",
    "clothing_store", "insurance_agency", "post_office", "train_station",
    "transit_station", "car_repair", "car_wash", "book_store", "liquor_store",
];

/// Returns true if the business is likely a franchise / big chain.
/// `types` is the Google Places `types` array; `name` the business name.
pub fn is_likely_franchise(name: &str, types: &[String]) -> bool {
    let n = name.to_lowercase();
    for chain in KNOWN_CHAINS {
        if n.contains(chain) {
            return true;
        }
    }
    let mut type_hits = 0;
    for t in types {
        let tl = t.to_lowercase();
        if CHAIN_TYPES.contains(&tl.as_str()) {
            type_hits += 1;
        }
    }
    // Strong multi-type signal: a place carrying 2+ chain-type tags is almost
    // certainly a big-box / corporate location, not a local small business.
    type_hits >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detets_known_chain_by_name() {
        assert!(is_likely_franchise("McDonald's", &[]));
        assert!(is_likely_franchise("Starbucks Coffee", &[]));
    }

    #[test]
    fn detets_local_business_as_not_franchise() {
        assert!(!is_likely_franchise("Joe's Barber Shop", &[]));
    }

    #[test]
    fn detets_chain_by_multiple_types() {
        let types = vec!["supermarket".to_string(), "department_store".to_string()];
        assert!(is_likely_franchise("Generic Market", &types));
    }

    #[test]
    fn single_type_is_not_enough() {
        let types = vec!["restaurant".to_string()];
        assert!(!is_likely_franchise("Local Eatery", &types));
    }
}
