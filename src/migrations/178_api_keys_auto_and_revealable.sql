-- 178 — account API keys: auto-provisioned, RETRIEVABLE, auditable (kanban t_7863696b).
--
-- David, 2026-11-11: every account is created with a key, and the key stays retrievable.
-- Stored twice from now on:
--   * key_hash      — SHA-256 hex, the ONLY thing authentication reads (routes.rs / auth middleware);
--   * key_encrypted — the same key sealed with the fleet provider-key envelope (`enc:v1:…`,
--                     pgp_sym_encrypt under PROVIDER_KEY_ENC_SECRET), used ONLY for display.
-- Authentication never decrypts; reveal never authenticates.

ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS key_encrypted text;

ALTER TABLE api_keys DROP CONSTRAINT IF EXISTS api_keys_key_encrypted_envelope;
ALTER TABLE api_keys ADD CONSTRAINT api_keys_key_encrypted_envelope
    CHECK (key_encrypted IS NULL OR key_encrypted LIKE 'enc:v1:%');

-- Every reveal is logged: who, when, which key (prefix only — never the secret).
CREATE TABLE IF NOT EXISTS api_key_reveals (
    id          uuid PRIMARY KEY,
    key_id      uuid NOT NULL,
    tenant_id   uuid,
    user_id     uuid,
    key_prefix  text,
    ip_address  text,
    user_agent  text,
    revealed_at timestamp with time zone NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_api_key_reveals_key
    ON api_key_reveals (key_id, revealed_at DESC);

-- One-time data backfills. A backfill that already ran must never run twice: a customer who
-- deliberately revoked every key must not have one silently re-minted on the next boot.
CREATE TABLE IF NOT EXISTS fleet_data_backfills (
    name       text PRIMARY KEY,
    applied_at timestamp with time zone NOT NULL DEFAULT NOW(),
    detail     text
);
