-- Seed a default password_reset email template for business owner logins
-- Use a function to avoid semicolon splitting in the HTML body
CREATE OR REPLACE FUNCTION seed_password_reset_template() RETURNS void AS $$
BEGIN
    INSERT INTO email_templates (name, subject, body, body_text, variables, category, directory_id)
    SELECT
        'password_reset',
        'Password Reset Request -- {{directory_name}}',
        '<!DOCTYPE html>
<html><head><meta charset="utf-8"></head>
<body style="font-family:Arial,sans-serif;max-width:480px;margin:40px auto;padding:20px;">
<div style="background:#f8f9fa;border-radius:12px;padding:32px;text-align:center;">
  <h1 style="color:#1e293b;margin:0 0 8px;">Password Reset</h1>
  <p style="color:#64748b;font-size:14px;margin-bottom:24px;">
    A password reset was requested for your <strong>{{directory_name}}</strong> account.
    Use the code below to reset your password. It expires in 1 hour.
  </p>
  <div style="background:#fff;border:2px dashed #6366f1;border-radius:8px;padding:16px 24px;margin:0 auto 24px;display:inline-block;">
    <code style="font-size:28px;font-weight:700;letter-spacing:4px;color:#6366f1;">{{code}}</code>
  </div>
  <p style="color:#94a3b8;font-size:12px;">If you did not request this, you can safely ignore this email.</p>
</div>
<p style="text-align:center;color:#94a3b8;font-size:11px;margin-top:16px;">{{directory_name}} -- Powered by SwiftSoftware</p>
</body></html>',
        'Password Reset

Your reset code is: {{code}}

This code expires in 1 hour.
If you did not request this, ignore this email.

- {{directory_name}}
- Multi-Directory',
        ARRAY['code','token','directory_name']::text[],
        'auth',
        NULL
    WHERE NOT EXISTS (
        SELECT 1 FROM email_templates WHERE name = 'password_reset' AND directory_id IS NULL
    );
END;
$$ LANGUAGE plpgsql;

SELECT seed_password_reset_template();
DROP FUNCTION seed_password_reset_template();
