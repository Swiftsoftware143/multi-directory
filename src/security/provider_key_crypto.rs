//! At-rest encryption for BYOK provider credentials (`provider_keys.api_key`).
//!
//! Threat model
//! ------------
//! `provider_keys.api_key` holds CUSTOMER-SUPPLIED third-party credentials (Google Places,
//! DataForSEO, OpenAI/Anthropic, Resend/SendGrid, social tokens). They are money-bearing: a
//! database dump, a leaked backup or a read-only SQL grant must not hand them over.
//!
//! The master key therefore lives ONLY in the app environment (`PROVIDER_KEY_ENC_SECRET`) and
//! never in the database, so ciphertext at rest is useless without the process environment.
//! The previous design encrypted a COPY of the value into `provider_keys.api_key_encrypted`
//! with a key read from `app_encryption_config` — the plaintext column stayed populated and
//! the key sat in the same database as the ciphertext, so a dump defeated it. Migration 095
//! removes that trigger.
//!
//! Ciphertext format
//! -----------------
//!     enc:v1:<base64( pgp_sym_encrypt(plaintext, master_key) )>
//!
//! * AES-256 (pgcrypto PGP symmetric, `cipher-algo=aes256`), random salt per write, so the
//!   same value encrypts differently every time. The base64 payload is single-line (the
//!   encoder's 76-char line wrapping is stripped) so the column never holds a multi-line
//!   secret.
//! * The `enc:v1:` prefix is self-describing and is what the DB CHECK constraint (migration
//!   095) enforces, so a future writer that forgets to encrypt FAILS CLOSED instead of
//!   silently storing a plaintext credential.
//! * A value without the prefix is a legacy plaintext row (pre-2026-09-21); it is read
//!   through unchanged so nothing breaks before the one-off backfill has run.
//!
//! Fail-closed rule
//! ----------------
//! A missing / too-short master key makes [`encrypt_for_storage`] return `NotConfigured`. It
//! NEVER falls back to storing the plaintext. Masking of read paths is unaffected: ciphertext
//! is never returned to a client, only a `sk-...xyz` mask of the DECRYPTED value.

use sqlx::PgPool;
use std::sync::OnceLock;

/// Marker that identifies an encrypted value inside `provider_keys.api_key`.
pub const ENC_PREFIX: &str = "enc:v1:";

/// pgcrypto options: AES-256, no compression (no compression side channels on secrets),
/// s2k mode 3 (iterated + salted, the OpenPGP default) so every write salts the key.
const PGP_OPTIONS: &str = "cipher-algo=aes256, compress-algo=0, s2k-mode=3";

/// Refuse weak master keys: anything shorter than this is treated as "not configured".
const MIN_MASTER_KEY_LEN: usize = 32;

const ENV_VAR: &str = "PROVIDER_KEY_ENC_SECRET";

static MASTER_KEY: OnceLock<Option<String>> = OnceLock::new();

#[derive(Debug)]
pub enum CryptoError {
    /// `PROVIDER_KEY_ENC_SECRET` is missing or too weak — writes must fail rather than
    /// store a credential in the clear.
    NotConfigured,
    /// The database rejected the operation (e.g. decryption with a wrong master key).
    Database(sqlx::Error),
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::NotConfigured => write!(
                f,
                "provider key encryption is not configured ({} missing or shorter than {} chars)",
                ENV_VAR, MIN_MASTER_KEY_LEN
            ),
            CryptoError::Database(e) => write!(f, "provider key crypto database error: {}", e),
        }
    }
}

impl std::error::Error for CryptoError {}

impl From<sqlx::Error> for CryptoError {
    fn from(e: sqlx::Error) -> Self {
        CryptoError::Database(e)
    }
}

impl From<CryptoError> for crate::error::AppError {
    fn from(e: CryptoError) -> Self {
        match e {
            CryptoError::NotConfigured => {
                crate::error::AppError::Internal(format!("cannot store provider key: {}", e))
            }
            CryptoError::Database(e) => crate::error::AppError::Database(e),
        }
    }
}

/// The master key, read from the environment once per process.
///
/// Returns `None` (and logs once) when the variable is missing or too short — callers must
/// treat that as fail-closed, never as "use no encryption".
pub fn master_key() -> Option<&'static str> {
    MASTER_KEY
        .get_or_init(|| match std::env::var(ENV_VAR) {
            Ok(v) if v.len() >= MIN_MASTER_KEY_LEN => Some(v),
            Ok(v) => {
                tracing::error!(
                    len = v.len(),
                    min = MIN_MASTER_KEY_LEN,
                    "{} is too short — provider key writes will fail closed",
                    ENV_VAR
                );
                None
            }
            Err(_) => {
                tracing::error!(
                    "{} is not set — provider key writes will fail closed (a plaintext credential is never stored)",
                    ENV_VAR
                );
                None
            }
        })
        .as_deref()
}

/// Whether at-rest encryption is configured (used for the boot-time log line).
pub fn is_configured() -> bool {
    master_key().is_some()
}

/// Encrypt a provider credential for storage. Fail-closed: without a master key this errors
/// instead of returning the plaintext.
pub async fn encrypt_for_storage(db: &PgPool, plaintext: &str) -> Result<String, CryptoError> {
    let key = master_key().ok_or(CryptoError::NotConfigured)?;
    if plaintext.is_empty() {
        // An empty slot is not a credential; keep it empty rather than encrypting nothing.
        return Ok(String::new());
    }

    // chr(10) is stripped because pgcrypto's base64 encoder line-wraps at 76 chars; the stored
    // value must stay a single line. decode() tolerates whitespace, so a value written before
    // this change still decrypts.
    let b64: String = sqlx::query_scalar(
        "SELECT replace(encode(pgp_sym_encrypt($1::text, $2::text, $3), 'base64'), chr(10), '')",
    )
    .bind(plaintext)
    .bind(key)
    .bind(PGP_OPTIONS)
    .fetch_one(db)
    .await?;

    Ok(format!("{}{}", ENC_PREFIX, b64))
}

/// Decrypt a stored provider credential.
///
/// A value without [`ENC_PREFIX`] is a legacy plaintext row and is returned unchanged.
pub async fn decrypt_from_storage(db: &PgPool, stored: &str) -> Result<String, CryptoError> {
    if !stored.starts_with(ENC_PREFIX) {
        return Ok(stored.to_string());
    }

    let key = master_key().ok_or(CryptoError::NotConfigured)?;
    let b64 = &stored[ENC_PREFIX.len()..];

    let plaintext: String =
        sqlx::query_scalar("SELECT pgp_sym_decrypt(decode($1, 'base64'), $2::text)")
            .bind(b64)
            .bind(key)
            .fetch_one(db)
            .await?;

    Ok(plaintext)
}

/// Read-for-USE: the decrypted credential, or `None`.
///
/// A key that cannot be decrypted (missing/rotated master key) must never leak the ciphertext
/// to a caller that would put it on the wire, so this logs and degrades to `None` — the same
/// "log + skip, never panic" contract the integration paths use for an unconfigured provider.
pub async fn decrypt_for_use(db: &PgPool, stored: &str, provider: &str) -> Option<String> {
    match decrypt_from_storage(db, stored).await {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::error!(
                provider = provider,
                error = %e,
                "provider key could not be decrypted — treating provider as unconfigured"
            );
            None
        }
    }
}

/// Read-for-DISPLAY: the decrypted credential, or an empty string.
///
/// Used by the masked list/get paths: on failure the caller renders "no usable key" rather
/// than the stored ciphertext.
pub async fn decrypt_for_display(db: &PgPool, stored: &str) -> String {
    match decrypt_from_storage(db, stored).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "provider key could not be decrypted for display");
            String::new()
        }
    }
}
