use std::env;
use serde_json::json;
use sqlx::PgPool;

/// Send a password reset email — tries the DB template first, falls back to inline.
/// Uses environment variables for provider configuration.
pub async fn send_reset_email(db: &PgPool, to: &str, token: &str) -> Result<(), String> {
    let api_url = env::var("EMAIL_API_URL")
        .map_err(|_| "EMAIL_API_URL not set".to_string())?;
    let api_key = env::var("EMAIL_API_KEY")
        .map_err(|_| "EMAIL_API_KEY not set".to_string())?;
    let from = env::var("EMAIL_FROM")
        .unwrap_or_else(|_| "admin@multidirectory.net".to_string());

    // Try to load a DB template for password_reset
    let (subject, html_body, text_body) = match sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT subject, body, body_text FROM email_templates \
         WHERE name = 'password_reset' AND directory_id IS NULL \
         ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(db)
    .await
    {
        Ok(Some(t)) => {
            // Replace template variables
            let subject = t.0.replace("{{token}}", token).replace("{{code}}", token);
            let html = t.1.replace("{{token}}", token).replace("{{code}}", token);
            let text = t.2.map(|t| t.replace("{{token}}", token).replace("{{code}}", token));
            (subject, html, text)
        }
        _ => {
            // Fallback to inline template
            let subject = "Password Reset Request — Multi-Directory".to_string();
            let html = format!(
                r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"></head>
<body style="font-family:Arial,sans-serif;max-width:480px;margin:40px auto;padding:20px;">
<div style="background:#f8f9fa;border-radius:12px;padding:32px;text-align:center;">
  <h1 style="color:#1e293b;margin:0 0 8px;">Password Reset</h1>
  <p style="color:#64748b;font-size:14px;margin-bottom:24px;">Use the code below to reset your password. It expires in 1 hour.</p>
  <div style="background:#fff;border:2px dashed #6366f1;border-radius:8px;padding:16px 24px;margin:0 auto 24px;display:inline-block;">
    <code style="font-size:24px;font-weight:700;letter-spacing:4px;color:#6366f1;">{}</code>
  </div>
  <p style="color:#94a3b8;font-size:12px;">If you didn't request this, you can safely ignore this email.</p>
</div>
<p style="text-align:center;color:#94a3b8;font-size:11px;margin-top:16px;">Multi-Directory — Powered by SwiftSoftware</p>
</body></html>"#,
                token
            );
            let text = format!(
                "Password Reset\n\nYour reset code is: {}\n\nThis code expires in 1 hour.\nIf you didn't request this, ignore this email.\n\n- Multi-Directory",
                token
            );
            (subject, html, Some(text))
        }
    };

    let payload = json!({
        "from": from,
        "to": to,
        "subject": subject,
        "html": html_body,
        "text": text_body,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send email request: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Email API returned {}: {}", status, text));
    }

    Ok(())
}
