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
use uuid::Uuid;

/// Base URL of the in-house email service (loopback only).
fn service_url() -> String {
    std::env::var("EMAIL_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3456".to_string())
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
            // B119: render through the SHARED merge-field engine so every field the save guard
            // allows ({{directory_name}}, {{contact_email}}, …) actually resolves at send time,
            // and an unresolved token is DROPPED — never mailed to the recipient as raw braces.
            let mut ctx = crate::merge_fields::MergeContext::for_network(db).await;
            ctx.set("token", token);
            ctx.set("code", token);
            ctx.set("year", chrono::Utc::now().format("%Y").to_string());
            let base = ctx
                .get("site_url")
                .unwrap_or("https://zaarhub.com")
                .to_string();
            ctx.set(
                "reset_link",
                format!("{}/reset?token={}", base.trim_end_matches('/'), token),
            );

            let r_subject = crate::merge_fields::render(&subj, &ctx);
            let r_html = crate::merge_fields::render(&html_src, &ctx);
            let r_text = text_src.map(|t| crate::merge_fields::render(&t, &ctx).text);

            // Anything the field set could not bind is drift, and a half-substituted email is
            // a defect, not a silent default: NAME it, loudly (t_ba93aea4).
            let mut missing: Vec<String> = Vec::new();
            for name in r_subject.missing.iter().chain(r_html.missing.iter()) {
                if !missing.contains(name) {
                    missing.push(name.clone());
                }
            }
            if !missing.is_empty() {
                tracing::warn!(
                    placeholders = %missing.join(", "),
                    "password_reset email template placeholder(s) had no value and were omitted — \
                     fix the row in the admin panel so a customer never sees a raw field name",
                );
            }

            (r_subject.text, r_html.text, r_text)
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

/// Card B76 — the two-step claim verification mail.
///
/// `link` is the absolute confirm URL; when the claim was AUTO-accepted, `temp_password` carries
/// the dashboard credentials so the same mail both proves ownership and hands over the login.
/// Uses the DB template `claim_verification` when the admin has configured one; otherwise an
/// inline fallback. Fails gracefully (an unconfigured transport is an Err the caller logs — the
/// claim row is already saved and the token stays valid).
pub async fn send_claim_verification_email(
    db: &PgPool,
    directory_id: Option<Uuid>,
    to: &str,
    business_name: &str,
    link: &str,
    temp_password: Option<&str>,
) -> Result<(), String> {
    let (subject, html, text) = match crate::handlers::email::resolve_system_template(
        db,
        "claim_verification",
        None,
    )
    .await
    {
        Some((subj, html_src, text_src)) => {
            let mut ctx = crate::merge_fields::MergeContext::for_network(db).await;
            ctx.set("business_name", business_name);
            ctx.set("verify_link", link);
            ctx.set("link", link);
            ctx.set("email", to);
            ctx.set_opt("temp_password", temp_password.map(str::to_string));
            let r_subject = crate::merge_fields::render(&subj, &ctx);
            let r_html = crate::merge_fields::render(&html_src, &ctx);
            let r_text = text_src.map(|t| crate::merge_fields::render(&t, &ctx).text);
            (r_subject.text, r_html.text, r_text)
        }
        None => {
            let creds_block = match temp_password {
                Some(p) => format!(
                    "<p style=\"color:#1e293b;font-size:14px;\">Your account is ready. Sign in with this temporary password and change it after your first login:</p>\
                     <div style=\"background:#fff;border:2px dashed #6366f1;border-radius:8px;padding:12px 20px;display:inline-block;margin:8px 0 16px;\"><code style=\"font-size:20px;font-weight:700;color:#6366f1;\">{p}</code></div>"
                ),
                None => "<p style=\"color:#1e293b;font-size:14px;\">We could not match your email domain to the listing automatically. Confirm you own this business by clicking below.</p>".to_string(),
            };
            let subject = format!("Confirm your listing: {business_name}");
            let html = format!(
                r#"<!DOCTYPE html><html><head><meta charset="utf-8"></head>
<body style="font-family:Arial,sans-serif;max-width:480px;margin:40px auto;padding:20px;">
<div style="background:#f8f9fa;border-radius:12px;padding:32px;text-align:center;">
  <h1 style="color:#1e293b;margin:0 0 8px;">Confirm your listing</h1>
  <p style="color:#64748b;font-size:14px;margin:0 0 16px;">{business_name}</p>
  {creds_block}
  <p style="margin:24px 0;"><a href="{link}" style="background:#6366f1;color:#fff;text-decoration:none;padding:12px 24px;border-radius:8px;font-weight:600;">Confirm ownership</a></p>
  <p style="color:#94a3b8;font-size:12px;word-break:break-all;">{link}</p>
</div>
<p style="text-align:center;color:#94a3b8;font-size:11px;margin-top:16px;">Multi-Directory — Powered by SwiftSoftware</p>
</body></html>"#
            );
            let text = match temp_password {
                Some(p) => format!(
                    "Confirm your listing: {business_name}\n\nTemporary password: {p}\n\nConfirm ownership: {link}\n\n- Multi-Directory"
                ),
                None => format!(
                    "Confirm your listing: {business_name}\n\nConfirm you own this business: {link}\n\n- Multi-Directory"
                ),
            };
            (subject, html, Some(text))
        }
    };

    send_rendered_email_for_directory(db, directory_id, to, &subject, &html, text.as_deref()).await
}

/// Card B76 — tell the platform admin a claim is waiting for MANUAL review (the domain did not
/// match, so it was not auto-accepted). Best-effort: an unconfigured transport logs and moves on.
pub async fn send_claim_review_notice(
    admin_email: &str,
    business_name: &str,
    claimant_email: &str,
    link: &str,
) -> Result<(), String> {
    let subject = format!("Claim needs review: {business_name}");
    let html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"></head>
<body style="font-family:Arial,sans-serif;max-width:520px;margin:40px auto;padding:20px;">
<div style="background:#fff7ed;border:1px solid #fdba74;border-radius:12px;padding:24px;">
  <h2 style="color:#9a3412;margin:0 0 8px;">A claim needs manual review</h2>
  <p style="color:#334155;font-size:14px;"><strong>{business_name}</strong> was claimed by <strong>{claimant_email}</strong>.</p>
  <p style="color:#334155;font-size:14px;">The email domain did not match the listing, so it was not auto-accepted. Review it in the admin panel.</p>
  <p style="margin:20px 0;"><a href="{link}" style="background:#6366f1;color:#fff;text-decoration:none;padding:10px 20px;border-radius:8px;font-weight:600;">Open the admin panel</a></p>
</div>
</body></html>"#
    );
    let text = format!(
        "Claim needs manual review\n\n{business_name} was claimed by {claimant_email}.\nReview: {link}\n"
    );
    send_rendered_email(admin_email, &subject, &html, Some(&text)).await
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

/// Directory-scoped system send (B91). Appends the directory's configured email signature (when
/// one is set) to the rendered message, then delivers it through the in-house service.
///
/// Every system mail that knows its directory goes through here so the admin's signature is
/// applied automatically — the behaviour this module documents — instead of only the reminder
/// path. A `None` directory (e.g. a platform login mail) is delivered unchanged.
pub async fn send_rendered_email_for_directory(
    db: &PgPool,
    directory_id: Option<Uuid>,
    to: &str,
    subject: &str,
    html: &str,
    text: Option<&str>,
) -> Result<(), String> {
    let (html, text) = match directory_id {
        Some(dir_id) => {
            let sig = crate::handlers::email::get_directory_signature(db, dir_id).await;
            crate::handlers::email::append_signature(html, text, &sig)
        }
        None => (html.to_string(), text.map(str::to_string)),
    };
    send_rendered_email(to, subject, &html, text.as_deref()).await
}
