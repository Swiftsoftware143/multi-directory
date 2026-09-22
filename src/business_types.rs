//! Single source of truth for the `businesses.business_type` taxonomy.
//!
//! Card B53 (2026-09-22): three places were supposed to agree on the allowed business
//! types and all three disagreed —
//!
//!   * the DB CHECK constraint `businesses_business_type_check` allowed
//!     local, supplier, distributor, wholesaler, farm, association, manufacturer, chain;
//!   * the register handler (`handlers::b2b::b2b_register`) accepted
//!     association, farm, wholesaler, distributor, manufacturer, **other**;
//!   * the register dropdowns offered yet another list, and the two register pages
//!     did not even agree with each other.
//!
//! Consequences, all live: `business_type=other` passed the handler, was rejected by the
//! constraint, and answered HTTP 500 **after** the `visitor_accounts` row had already been
//! committed (an orphaned account with no business); `supplier`, `local` and `chain` were
//! refused by the handler even though the database accepts them (so registering as a
//! SUPPLIER was impossible); and `manufacturer` registered 201 but was missing from every
//! supplier query, so every supplier endpoint answered 404.
//!
//! `BUSINESS_TYPES` below is the canonical list. Keep the three consumers aligned to it:
//!   * migration `100_business_types_single_source.sql` rebuilds the DB constraint from it;
//!   * `handlers::b2b::b2b_register` validates against `is_valid`;
//!   * `GET /api/v1/b2b/business-types` (`list_business_types`) serves it to the register UI,
//!     which also carries the same list as a no-JS fallback.
//! `the_taxonomy_is_self_consistent` unit test guards the invariants.

/// Every value the `businesses.business_type` CHECK constraint accepts — nothing more.
pub const BUSINESS_TYPES: &[&str] = &[
    "local",
    "supplier",
    "distributor",
    "wholesaler",
    "farm",
    "association",
    "manufacturer",
    "chain",
    "other",
];

/// The subset that owns the B2B supplier back-office (`/supplier/*`, `/b2b/orders`,
/// `/b2b/products/my`) and that supplier discovery / marketplace queries search.
///
/// `manufacturer` used to be missing here, which is what made a manufacturer account
/// register successfully and then 404 on every supplier endpoint.
pub const SUPPLIER_TYPES: &[&str] = &[
    "supplier",
    "distributor",
    "wholesaler",
    "farm",
    "association",
    "manufacturer",
];

/// Is this a business type the database will accept? Case-insensitive.
pub fn is_valid(business_type: &str) -> bool {
    let t = business_type.trim().to_ascii_lowercase();
    BUSINESS_TYPES.contains(&t.as_str())
}

/// Canonical lowercase form of a caller-supplied business type.
pub fn normalize(business_type: &str) -> String {
    business_type.trim().to_ascii_lowercase()
}

/// `'a','b','c'` — for embedding in an `IN (...)` clause in a Rust raw string.
///
/// Never build `$N` placeholders out of this (this repo had 94 broken `\x24N`
/// placeholders inside `r#"…"#` strings); these are literals from a compile-time
/// constant, not user input.
pub fn sql_in_list(types: &[&str]) -> String {
    types
        .iter()
        .map(|t| format!("'{}'", t))
        .collect::<Vec<_>>()
        .join(",")
}

/// `SUPPLIER_TYPES` as an owned vector, for `AND business_type = ANY($n)` binds.
///
/// Prefer this over `sql_in_list` in query text: `= ANY($n)` needs no literal list in the
/// SQL string at all, so a raw string cannot accidentally swallow a placeholder and the
/// taxonomy lives in exactly one place.
pub fn supplier_types_param() -> Vec<String> {
    SUPPLIER_TYPES.iter().map(|s| s.to_string()).collect()
}

/// The canonical list as JSON, for the register UI.
pub fn as_json() -> serde_json::Value {
    serde_json::json!({
        "business_types": BUSINESS_TYPES,
        "supplier_types": SUPPLIER_TYPES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taxonomy_is_self_consistent() {
        // No duplicates: a duplicate would silently shrink the IN(...) list.
        let mut sorted = BUSINESS_TYPES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            BUSINESS_TYPES.len(),
            "duplicate business type"
        );

        // 'other' must be registerable — it is offered by the UI and was a live 500.
        assert!(is_valid("other"));

        // The three values the handler used to refuse although the DB accepts them.
        for t in ["supplier", "local", "chain"] {
            assert!(is_valid(t), "{} must be registerable", t);
        }

        // Every supplier type must also be a business type (otherwise a supplier could
        // never be registered at all).
        for t in SUPPLIER_TYPES {
            assert!(
                BUSINESS_TYPES.contains(t),
                "supplier type {} is not a business type",
                t
            );
        }

        // manufacturer must stay a supplier type: its absence caused 404s on every
        // supplier endpoint for manufacturer accounts.
        assert!(SUPPLIER_TYPES.contains(&"manufacturer"));

        // Case-insensitivity, because the handler lowercases before validating.
        assert!(is_valid("SUPPLIER"));
        assert_eq!(normalize("  Supplier "), "supplier");
        assert!(!is_valid("not-a-type"));

        // The embedded SQL literal list must quote every value (an unquoted value would
        // make the IN(...) clause a column reference instead of a literal).
        let list = sql_in_list(SUPPLIER_TYPES);
        assert_eq!(list.matches('\'').count(), SUPPLIER_TYPES.len() * 2);
        assert!(list.starts_with("'supplier','distributor'"));
    }
}
