-- 096 the remaining *_encrypted credential columns must never hold plaintext at rest.
--
-- Siblings of migration 095, which did the same for provider_keys.api_key:
--   connected_services.api_key_encrypted        customer-supplied IncentiveSwift API key
--   payment_providers.api_key_encrypted         payment gateway secret key (Stripe/PayPal/Square)
--   payment_providers.webhook_secret_encrypted  the gateway's webhook signing secret
--
-- The columns were NAMED encrypted and held the raw request value: no trigger encrypted them and
-- the handlers bound the request body straight into the INSERT/UPDATE. The app now encrypts through
-- the single choke point (src/security/provider_key_crypto.rs, enc:v1 + AES-256 pgcrypto under the
-- PROVIDER_KEY_ENC_SECRET environment variable only, fail-closed).
--
-- These CHECK constraints make the guarantee DB-enforced instead of code-enforced: a future writer
-- that forgets to encrypt is REJECTED here rather than quietly storing a credential in the clear.
-- Both tables are empty today, so there is no backfill and the constraint validates immediately.
--
-- Added NOT VALID then validated, so an existing row can never make the migration fail.

ALTER TABLE connected_services DROP CONSTRAINT IF EXISTS connected_services_api_key_encrypted_check;

ALTER TABLE connected_services ADD CONSTRAINT connected_services_api_key_encrypted_check
  CHECK (api_key_encrypted IS NULL OR api_key_encrypted = '' OR api_key_encrypted LIKE 'enc:v1:%') NOT VALID;

ALTER TABLE connected_services VALIDATE CONSTRAINT connected_services_api_key_encrypted_check;

ALTER TABLE payment_providers DROP CONSTRAINT IF EXISTS payment_providers_api_key_encrypted_check;

ALTER TABLE payment_providers ADD CONSTRAINT payment_providers_api_key_encrypted_check
  CHECK (api_key_encrypted IS NULL OR api_key_encrypted = '' OR api_key_encrypted LIKE 'enc:v1:%') NOT VALID;

ALTER TABLE payment_providers VALIDATE CONSTRAINT payment_providers_api_key_encrypted_check;

ALTER TABLE payment_providers DROP CONSTRAINT IF EXISTS payment_providers_webhook_secret_encrypted_check;

ALTER TABLE payment_providers ADD CONSTRAINT payment_providers_webhook_secret_encrypted_check
  CHECK (webhook_secret_encrypted IS NULL OR webhook_secret_encrypted = '' OR webhook_secret_encrypted LIKE 'enc:v1:%') NOT VALID;

ALTER TABLE payment_providers VALIDATE CONSTRAINT payment_providers_webhook_secret_encrypted_check;
