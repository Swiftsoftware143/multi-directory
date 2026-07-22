-- Migration: Encrypt provider_keys at rest
-- Applies pgp_sym_encrypt to all stored API keys

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Encryption config table
CREATE TABLE IF NOT EXISTS app_encryption_config (
    id SERIAL PRIMARY KEY,
    active BOOLEAN NOT NULL DEFAULT true,
    encryption_key TEXT NOT NULL DEFAULT gen_random_uuid()::text,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    rotated_at TIMESTAMPTZ
);

INSERT INTO app_encryption_config (encryption_key)
SELECT gen_random_uuid()::text
WHERE NOT EXISTS (SELECT 1 FROM app_encryption_config WHERE active = true);

-- Add encrypted columns to provider_keys
ALTER TABLE provider_keys ADD COLUMN IF NOT EXISTS api_key_encrypted BYTEA;
ALTER TABLE provider_keys ADD COLUMN IF NOT EXISTS base_url_encrypted BYTEA;

-- Encrypt existing plaintext keys
UPDATE provider_keys pk
SET api_key_encrypted = pgp_sym_encrypt(pk.api_key, (SELECT encryption_key FROM app_encryption_config WHERE active = true LIMIT 1)),
    base_url_encrypted = CASE WHEN pk.base_url IS NOT NULL AND pk.base_url != '' 
        THEN pgp_sym_encrypt(pk.base_url, (SELECT encryption_key FROM app_encryption_config WHERE active = true LIMIT 1))
        ELSE NULL END
WHERE pk.api_key IS NOT NULL AND pk.api_key != '';

-- Trigger to auto-encrypt on INSERT/UPDATE
CREATE OR REPLACE FUNCTION encrypt_provider_key() RETURNS trigger AS $$
DECLARE
    enc_key TEXT;
BEGIN
    SELECT encryption_key INTO enc_key FROM app_encryption_config WHERE active = true LIMIT 1;
    IF enc_key IS NULL THEN
        RAISE EXCEPTION 'No active encryption config found';
    END IF;
    NEW.api_key_encrypted := pgp_sym_encrypt(NEW.api_key, enc_key);
    IF NEW.base_url IS NOT NULL AND NEW.base_url != '' THEN
        NEW.base_url_encrypted := pgp_sym_encrypt(NEW.base_url, enc_key);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_encrypt_provider_key ON provider_keys;
CREATE TRIGGER trg_encrypt_provider_key
    BEFORE INSERT OR UPDATE OF api_key, base_url ON provider_keys
    FOR EACH ROW
    EXECUTE FUNCTION encrypt_provider_key();

-- Decrypt helper function
CREATE OR REPLACE FUNCTION decrypt_provider_key(encrypted_data BYTEA) RETURNS TEXT AS $$
DECLARE
    enc_key TEXT;
BEGIN
    SELECT encryption_key INTO enc_key FROM app_encryption_config WHERE active = true LIMIT 1;
    IF enc_key IS NULL THEN
        RAISE EXCEPTION 'No active encryption config found';
    END IF;
    RETURN pgp_sym_decrypt(encrypted_data, enc_key);
END;
$$ LANGUAGE plpgsql;
