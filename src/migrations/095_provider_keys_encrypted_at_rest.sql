-- 095 provider_keys.api_key holds BYOK credentials and must never be plaintext at rest.
--
-- The app encrypts it with pgp_sym_encrypt under a master key that lives ONLY in the process
-- environment (PROVIDER_KEY_ENC_SECRET) and stores the result as enc:v1 base64 text.
--
-- Removed here: the DB-side trigger that copied api_key into provider_keys.api_key_encrypted
-- using a key read from app_encryption_config. It left the plaintext column fully populated
-- and keyed the ciphertext with a secret stored in the same database, so a dump defeated it.

DROP TRIGGER IF EXISTS trg_encrypt_provider_key ON provider_keys;

ALTER TABLE provider_keys DROP CONSTRAINT IF EXISTS provider_keys_api_key_encrypted;

ALTER TABLE provider_keys ADD CONSTRAINT provider_keys_api_key_encrypted
  CHECK (api_key = '' OR api_key LIKE 'enc:v1:%') NOT VALID;
