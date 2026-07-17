-- Email template: add plain-text body column
ALTER TABLE email_templates ADD COLUMN IF NOT EXISTS body_text TEXT;

-- Directory: add email signature fields
ALTER TABLE directories ADD COLUMN IF NOT EXISTS email_signature_html TEXT;
ALTER TABLE directories ADD COLUMN IF NOT EXISTS email_signature_text TEXT;
