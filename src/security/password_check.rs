//! Credential verification with exactly ONE failure mode: "not this password".
//!
//! A stored hash that argon2 cannot PARSE — a placeholder, a truncated column, a row written by an
//! import or a legacy migration — is a DATA defect, but it is never a correct password. Every login
//! surface used to map that parse failure to `AppError::Hash`, so such a row answered
//! `500 Password hashing error` and handed internal error text to an anonymous caller (measured
//! 2026-10-30 through the public edge: `POST /api/v1/auth/login` for the fleet canary rows, whose
//! hash is the literal sentinel `probe-not-a-…`, answered 500 while a normal wrong password answers
//! 401 — the endpoint silently told the caller which accounts are damaged, and any real row whose
//! hash got truncated crashed the login instead of refusing it).
//!
//! Asking this one function makes the answer identical to a wrong password. The defect itself is
//! still raised for ops, at `error`, without ever logging the hash.

use argon2::{Argon2, PasswordHash, PasswordVerifier};

/// Does `given` match `stored_hash`? `false` for a mismatch AND for a hash that cannot be parsed;
/// the unparseable case is logged so the stored row is not silently unusable.
pub fn matches(stored_hash: &str, given: &str) -> bool {
    let parsed = match PasswordHash::new(stored_hash) {
        Ok(p) => p,
        Err(e) => {
            // No hash, no address, no password in this line — the class of defect plus how to find
            // the rows is what ops needs. `{e}` is argon2's own shape complaint ("password hash
            // string missing field"), never key material.
            tracing::error!(
                "stored password hash is not parseable ({e}) — refusing the login as invalid \
                 credentials; find the row with: SELECT id, email FROM <table> \
                 WHERE password_hash NOT LIKE '$argon2%'"
            );
            return false;
        }
    };
    Argon2::default()
        .verify_password(given.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::PasswordHasher;

    #[test]
    fn a_real_hash_matches_only_its_own_password() {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"ProbePass123!", &salt)
            .unwrap()
            .to_string();
        assert!(matches(&hash, "ProbePass123!"));
        assert!(!matches(&hash, "ProbePass123"));
        assert!(!matches(&hash, ""));
    }

    #[test]
    fn an_unparseable_hash_is_a_refusal_not_a_panic() {
        // The measured sentinel from the fleet canary rows.
        assert!(!matches("probe-not-a-real-hash", "ProbePass123!"));
        assert!(!matches("", ""));
        assert!(!matches("$argon2id$truncated", "x"));
    }
}
