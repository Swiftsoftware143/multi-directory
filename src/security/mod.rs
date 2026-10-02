//! At-rest protection for secrets the app must be able to REPLAY (BYOK credentials).
pub mod provider_key_crypto;

// Email-address syntax check shared by every boundary that writes a login identity
// (`users.email`, `visitor_accounts.email`).
pub mod email_addr;
