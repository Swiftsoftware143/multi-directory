//! At-rest protection for secrets the app must be able to REPLAY (BYOK credentials).
pub mod provider_key_crypto;

// Default-deny routing (kanban t_8bfbbf7f): the committed allowlist of routes that may answer a
// caller with no credential. Everything else is private.
pub mod route_policy;

// Email-address syntax check shared by every boundary that writes a login identity
// (`users.email`, `visitor_accounts.email`).
pub mod email_addr;

// Credential verification that answers "invalid credentials" — never a 500 — when the STORED hash
// cannot be parsed (measured live 2026-10-30: a damaged row answered 500 at the login endpoint).
pub mod password_check;
