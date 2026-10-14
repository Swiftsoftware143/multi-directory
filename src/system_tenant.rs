//! The platform's own tenant id, in ONE place (kanban t_9e938ad1, gate 5a).
//!
//! A UUID written into a handler is a defect: the row it names should be looked up by slug, not
//! copied (RULES Part 1, gate 5a). ONE id is structural and cannot be resolved at every call site:
//!
//! * `SYSTEM_TENANT_ID` — the `swiftsoftware` tenant seeded by migration 001. Every
//!   platform-operated row hangs off it (provider keys, legal pages, portal accounts). It is
//!   declared here ONCE and is BOUND as a query parameter or compared as a typed `Uuid` at every
//!   use site — never interpolated into SQL text. A future change to the id is a one-line change.
//!
//! The all-zero "no tenant / platform scope" sentinel is deliberately NOT a constant: it is not a
//! tenant row at all, and Rust already spells it canonically as `uuid::Uuid::nil()`, so every use
//! site says `Uuid::nil()` and carries no literal.

use uuid::Uuid;

/// Slug of the platform's own tenant (seeded by migration 001).
pub const SYSTEM_TENANT_SLUG: &str = "swiftsoftware";

/// The `swiftsoftware` tenant id, seeded by migration 001.
pub const SYSTEM_TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";

/// [`SYSTEM_TENANT_ID`] as a typed `Uuid`.
pub fn system_tenant_uuid() -> Uuid {
    Uuid::parse_str(SYSTEM_TENANT_ID).expect("SYSTEM_TENANT_ID is a valid UUID constant")
}
