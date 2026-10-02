//! System email — password reset and other transactional mail.
//!
//! Round 5 (T1): the app no longer talks to a third-party email API through
//! server-wide env vars (`EMAIL_API_URL` / `EMAIL_API_KEY` are gone). Every
//! system message goes to the in-house email service on 127.0.0.1:3456, which
//! reads the transport + credentials for the directory from the database at send
//! time. The admin picks the transport in the panel; nothing is hardwired here.
//!
//! Fails gracefully: a down service or an unconfigured directory yields an Err
//! string that the caller logs — it never panics and never blocks the request.

use serde_json::json;
use sqlx::PgPool;

/// Base URL of the in-house email service (loopback only).
fn service_url() -> String {
    std::env::var("EMAIL_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3456".to_string())
}

/// `{{name}}`-shaped tokens that survived rendering — i.e. names this renderer failed
/// to substitute. `send_reset_email` binds exactly `{{token}}` and `{{code}}`; any other
/// double-brace name in the stored row is drift that would reach the recipient literally.
/// This is not hypothetical: the global default `password_reset` row shipped
/// `{{directory_name}}` in its subject, html and body_text and mailed it verbatim, with
/// zero log lines, until kanban t_ba93aea4. Naming the leftover is the half that makes
/// the next drift loud instead of silent.
fn unresolved_placeholders(rendered: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = rendered;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else { break };
        let name = after[..close].trim();
        if !name.is_empty()
            && name.len() < 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            && !out.contains(&name)
        {
            out.push(name);
        }
        rest = &after[close + 2..];
    }
    out
}

/// Every unresolved placeholder across the three rendered fields, deduped, in first-seen order.
fn unbound_placeholders<'a>(fields: &[(&'a str, &'a str)]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for (_, text) in fields {
        for name in unresolved_placeholders(text) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Send a password reset email — tries the DB template first, falls back to inline.
pub async fn send_reset_email(db: &PgPool, to: &str, token: &str) -> Result<(), String> {
    // Try to load a DB template for password_reset
    let (subject, html_body, text_body) = match crate::handlers::email::resolve_system_template(
        db,
        "password_reset",
        None,
    )
    .await
    {
        Some((subj, html_src, text_src)) => {
            // Replace template variables
            let subject = subj.replace("{{token}}", token).replace("{{code}}", token);
            let html = html_src
                .replace("{{token}}", token)
                .replace("{{code}}", token);
            let text = text_src.map(|t| t.replace("{{token}}", token).replace("{{code}}", token));

            // Anything the whitelist above could not bind is drift, and a half-substituted
            // email is a defect, not a silent default: NAME it, loudly (t_ba93aea4).
            let mut fields: Vec<(&str, &str)> =
                vec![("subject", subject.as_str()), ("html", html.as_str())];
            if let Some(body) = text.as_deref() {
                fields.push(("body_text", body));
            }
            let missing = unbound_placeholders(&fields);
            if !missing.is_empty() {
                tracing::warn!(
                    placeholders = %missing.join(", "),
                    "password_reset email template placeholder(s) left unsubstituted — the recipient would receive them literally. \
                     email_templates(name='password_reset') must only use {{token}} and {{code}}; fix the row in the admin panel",
                );
            }

            (subject, html, text)
        }
        None => {
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

    send_rendered_email(to, &subject, &html_body, text_body.as_deref()).await
}

/// Send an already-rendered message through the in-house email service. Returns Err with the
/// service's own words on failure; a "skipped" (unconfigured transport) is an Err because the
/// caller asked for mail and it was not delivered — it must never look like a success.
pub async fn send_rendered_email(
    to: &str,
    subject: &str,
    html: &str,
    text: Option<&str>,
) -> Result<(), String> {
    let payload = json!({
        "to": to,
        "subject": subject,
        "html": html,
        "text": text,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/send", service_url()))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Email service unreachable on {}: {}", service_url(), e))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Email service returned {}: {}", status, body));
    }
    // The service answers 200 with {"status":"sent"|"skipped"|"error"} — a skip is
    // not an error: the reset code is still valid, the admin just has not finished
    // configuring the transport.
    if body.contains("\"status\":\"sent\"") || body.contains("\"status\": \"sent\"") {
        Ok(())
    } else {
        tracing::warn!("[email] system mail not delivered: {}", body);
        Err(format!("Email service did not deliver: {}", body))
    }
}
