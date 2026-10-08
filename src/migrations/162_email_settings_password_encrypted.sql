-- 162: directory_email_settings.smtp_password holds a mail-transport credential and must never be
-- plaintext at rest.
--
-- The app encrypts it with pgp_sym_encrypt under the master key that lives ONLY in the process
-- environment (PROVIDER_KEY_ENC_SECRET) and stores the result as enc:v1 base64 text -- the same
-- at-rest path every other credential uses (src/security/provider_key_crypto.rs, migrations
-- 095/096). This CHECK makes a future writer that forgets to encrypt FAIL CLOSED instead of
-- silently storing a plaintext credential.
--
-- Added NOT VALID: the constraint is enforced on inserts/updates from here on, while a
-- pre-existing plaintext row is backfilled out-of-band (no code path reads or writes the value
-- except the email-settings handler, which now encrypts on write and decrypts on use). The
-- master key must never be coupled into the migrations directory, so the backfill is not here.

ALTER TABLE directory_email_settings
    DROP CONSTRAINT IF EXISTS directory_email_settings_smtp_password_encrypted;

ALTER TABLE directory_email_settings
    ADD CONSTRAINT directory_email_settings_smtp_password_encrypted
    CHECK (smtp_password = '' OR smtp_password LIKE 'enc:v1:%') NOT VALID;
