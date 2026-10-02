-- Migration 083: per-directory email transport choice (round 5, T1/T2).
--
-- "Nothing is hardwired": the directory admin picks the transport in the admin
-- panel (SMTP / Mailgun / SendGrid / Sendiio) and it is stored here, then read at
-- send time by the email service on 127.0.0.1:3456. Provider choice is NEVER
-- compiled into the app or baked into an image.
--
-- SMTP credentials live in the smtp_* columns below; API transports read their
-- key from provider_keys (default key first) at send time.

ALTER TABLE directory_email_settings
    ADD COLUMN IF NOT EXISTS transport TEXT NOT NULL DEFAULT 'smtp';

ALTER TABLE directory_email_settings
    DROP CONSTRAINT IF EXISTS directory_email_settings_transport_check;

ALTER TABLE directory_email_settings
    ADD CONSTRAINT directory_email_settings_transport_check
    CHECK (transport IN ('smtp', 'mailgun', 'sendgrid', 'sendiio'));
