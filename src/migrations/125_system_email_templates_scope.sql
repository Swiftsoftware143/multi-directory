-- 125: System email templates — network scope, activation flag and system-event mapping.
--
-- David (2026-10-02): the admin panel needs a "System Email Templates" tab where a template
-- can be bound to a system event (password reset, claim/verification code, signup
-- confirmation, statement, notification) and scoped to a NETWORK (inherited by its cities) or
-- to a standalone directory. Resolution in code is directory -> network -> global, most
-- recently updated first, active rows only.
--
-- Backfill: the existing global password_reset row is bound to the password_reset event so
-- today's reset mail keeps working through the new resolver.

ALTER TABLE email_templates
    ADD COLUMN IF NOT EXISTS network_id uuid REFERENCES networks(id) ON DELETE CASCADE;

ALTER TABLE email_templates
    ADD COLUMN IF NOT EXISTS event_key text;

ALTER TABLE email_templates
    ADD COLUMN IF NOT EXISTS is_active boolean NOT NULL DEFAULT true;

CREATE INDEX IF NOT EXISTS idx_email_templates_network_id
    ON email_templates(network_id);

CREATE INDEX IF NOT EXISTS idx_email_templates_event_key
    ON email_templates(event_key) WHERE event_key IS NOT NULL;

UPDATE email_templates
   SET event_key = 'password_reset'
 WHERE name = 'password_reset'
   AND event_key IS NULL;
