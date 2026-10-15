//! Dashboard reminder cron — branded, template-driven.
//!
//! - Looks up the directory's branded email template (category='dashboard_reminder')
//! - Falls back to a default template with the directory's brand colors
//! - Substitutes template variables: {business_name}, {directory_name}, {owner_name},
//!   {dashboard_url}, {primary_color}, {accent_color}, {text_color}, {background_color}
//! - Brand colors come from directory's color_scheme JSON (or network_branding)
//! - If no custom template exists, one is auto-created with the directory's brand

use chrono::{Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

const REMINDER_CATEGORY: &str = "dashboard_reminder";

#[derive(Debug, sqlx::FromRow)]
struct ReminderCandidate {
    contact_id: Uuid,
    directory_id: Uuid,
    directory_slug: String,
    owner_email: String,
    owner_name: Option<String>,
    business_name: String,
    business_slug: String,
}

#[derive(Debug, Clone)]
struct BrandColors {
    primary: String,
    accent: String,
    background: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct SmtpRequest {
    to: String,
    subject: String,
    html: String,
    text: String,
    from_name: String,
}

#[derive(Debug, sqlx::FromRow)]
struct EmailTemplate {
    subject: String,
    body: String,
    body_text: Option<String>,
}

/// Start the reminder cron — once per hour.
pub fn start_reminder_cron(db: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            tracing::info!("[reminders] Running dashboard reminder check...");
            if let Err(e) = run_reminder_check(&db).await {
                tracing::error!("[reminders] Check failed: {}", e);
            }
            tracing::info!("[reminders] Dashboard reminder check complete.");
        }
    });
}

async fn run_reminder_check(db: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let seven_days_ago = Utc::now() - Duration::days(7);

    let candidates = sqlx::query_as::<_, ReminderCandidate>(
        r#"
        SELECT
            c.id AS contact_id,
            d.id AS directory_id,
            d.slug AS directory_slug,
            cb.owner_email,
            cb.owner_name,
            b.name AS business_name,
            b.slug AS business_slug
        FROM claimed_businesses cb
        JOIN businesses b ON b.id = cb.business_id
        JOIN directories d ON d.id = b.directory_id
        JOIN crm_contacts c ON c.id = (
            SELECT id FROM crm_contacts
            WHERE directory_id = b.directory_id
            AND email = cb.owner_email
            LIMIT 1
        )
        WHERE cb.is_active = true
          AND (cb.last_dashboard_login IS NULL OR cb.last_dashboard_login < $1)
          AND (c.last_contacted_at IS NULL OR c.last_contacted_at < $1)
          AND b.is_active = true
        ORDER BY cb.last_dashboard_login ASC NULLS FIRST
        "#,
    )
    .bind(seven_days_ago)
    .fetch_all(db)
    .await?;

    if candidates.is_empty() {
        return Ok(());
    }

    tracing::info!("[reminders] Found {} candidate(s).", candidates.len());

    let mut template_cache: HashMap<Uuid, EmailTemplate> = HashMap::new();
    let mut brand_cache: HashMap<Uuid, BrandColors> = HashMap::new();
    let mut sent = 0u32;
    let mut failed = 0u32;

    for c in &candidates {
        if let std::collections::hash_map::Entry::Vacant(e) = brand_cache.entry(c.directory_id) {
            e.insert(resolve_brand_colors(db, c.directory_id).await);
        }
        let brand = &brand_cache[&c.directory_id];

        if let std::collections::hash_map::Entry::Vacant(e) = template_cache.entry(c.directory_id) {
            match load_or_create_template(db, c.directory_id, brand).await {
                Ok(t) => {
                    e.insert(t);
                }
                Err(e) => {
                    tracing::error!(
                        "[reminders] Template resolution failed for {}: {e}",
                        c.directory_slug
                    );
                    failed += 1;
                    continue;
                }
            }
        }
        let template = &template_cache[&c.directory_id];

        let dashboard_url = format!(
            "https://{}.workflowswift.com/my-business/{}",
            c.directory_slug, c.business_slug
        );
        let name = c.owner_name.as_deref().unwrap_or("Business Owner");

        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("business_name", c.business_name.clone());
        vars.insert("directory_name", c.directory_slug.clone());
        vars.insert("owner_name", name.to_string());
        vars.insert("dashboard_url", dashboard_url.clone());
        vars.insert("primary_color", brand.primary.clone());
        vars.insert("accent_color", brand.accent.clone());
        vars.insert("background_color", brand.background.clone());
        vars.insert("text_color", brand.text.clone());

        let subject = substitute(&template.subject, &vars);
        let html = substitute(&template.body, &vars);

        // Resolve directory signature
        let sig = lookup_signature(db, c.directory_id).await;
        let sig_html = sig.0.unwrap_or_default();
        let sig_text = sig.1.unwrap_or_default();

        // Append signature to HTML (insert before </body> or at end)
        let final_html = if !sig_html.is_empty() {
            if let Some(pos) = html.rfind("</body>") {
                let mut s = html;
                s.insert_str(pos, &sig_html);
                s
            } else {
                format!("{html}\n{sig_html}")
            }
        } else {
            html
        };

        // Build text version: use template body_text, fall back to auto-strip, append signature
        let template_text = template.body_text.as_deref().unwrap_or("");
        let base_text = if template_text.is_empty() {
            strip_html(&final_html)
        } else {
            substitute(template_text, &vars)
        };
        let final_text = if !sig_text.is_empty() {
            format!("{base_text}\n\n{sig_text}")
        } else {
            base_text
        };

        let smtp = SmtpRequest {
            to: c.owner_email.clone(),
            subject,
            html: final_html,
            text: final_text,
            from_name: c.directory_slug.clone(),
        };

        let client = reqwest::Client::new();
        match client
            .post("http://localhost:3456/send-email")
            .json(&smtp)
            .send()
            .await
        {
            Ok(resp) => {
                let st = resp.status();
                if st.is_success() {
                    let _ = sqlx::query(
                        "UPDATE crm_contacts SET last_contacted_at = NOW(), updated_at = NOW() WHERE id = $1"
                    )
                    .bind(c.contact_id)
                    .execute(db)
                    .await;
                    sent += 1;
                    tracing::info!(
                        "[reminders] Sent to {} ({})",
                        c.owner_email,
                        c.business_name
                    );
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!("[reminders] SMTP {st} for {}: {body}", c.owner_email);
                    failed += 1;
                }
            }
            Err(e) => {
                tracing::warn!("[reminders] SMTP unreachable for {}: {e}", c.owner_email);
                failed += 1;
            }
        };

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    tracing::info!("[reminders] Done: {sent} sent, {failed} failed.");
    Ok(())
}

/// A network's branding row, reduced to the four colours the reminder email needs.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
struct NetworkBrandRow {
    primary_color: Option<String>,
    accent_color: Option<String>,
    background_color: Option<String>,
    text_color: Option<String>,
}

/// Merge precedence for one directory's reminder-email palette: the directory's own
/// `color_scheme` value wins, the network's `network_branding` row fills any gap, and the
/// built-in default covers what is left. Split out of [`resolve_brand_colors`] so the
/// precedence is unit-testable without a database.
fn pick_colors(
    scheme: Option<&Value>,
    net: Option<&NetworkBrandRow>,
    default: &BrandColors,
) -> BrandColors {
    let from_scheme = |key: &str| {
        scheme
            .and_then(|s| s.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let from_net =
        |pick: fn(&NetworkBrandRow) -> &Option<String>| net.and_then(|n| pick(n).clone());
    BrandColors {
        primary: from_scheme("primary")
            .or_else(|| from_net(|n| &n.primary_color))
            .unwrap_or_else(|| default.primary.clone()),
        accent: from_scheme("accent")
            .or_else(|| from_net(|n| &n.accent_color))
            .unwrap_or_else(|| default.accent.clone()),
        background: from_scheme("background")
            .or_else(|| from_net(|n| &n.background_color))
            .unwrap_or_else(|| default.background.clone()),
        text: from_scheme("text")
            .or_else(|| from_net(|n| &n.text_color))
            .unwrap_or_else(|| default.text.clone()),
    }
}

async fn resolve_brand_colors(db: &PgPool, dir_id: Uuid) -> BrandColors {
    let default = BrandColors {
        primary: "#2563eb".into(),
        accent: "#f59e0b".into(),
        background: "#ffffff".into(),
        text: "#1e293b".into(),
    };

    let Ok(Some((scheme, network_id))) = sqlx::query_as::<_, (Option<Value>, Option<Uuid>)>(
        "SELECT d.color_scheme, d.network_id FROM directories d WHERE d.id = $1",
    )
    .bind(dir_id)
    .fetch_optional(db)
    .await
    else {
        return default;
    };

    // The network's branding row supplies every colour the directory's own scheme
    // does not pin down. Read it even when `color_scheme` is absent.
    let net: Option<NetworkBrandRow> = if let Some(nid) = network_id {
        sqlx::query_as::<_, NetworkBrandRow>(
            "SELECT primary_color, accent_color, background_color, text_color \
             FROM network_branding WHERE network_id = $1",
        )
        .bind(nid)
        .fetch_optional(db)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("[reminders] network branding lookup failed for {nid}: {e}");
            None
        })
    } else {
        None
    };

    pick_colors(scheme.as_ref(), net.as_ref(), &default)
}

async fn load_or_create_template(
    db: &PgPool,
    dir_id: Uuid,
    brand: &BrandColors,
) -> Result<EmailTemplate, Box<dyn std::error::Error>> {
    // Try directory-specific template first
    if let Some(t) = sqlx::query_as::<_, EmailTemplate>(
        "SELECT id, name, subject, body, body_text, variables FROM email_templates \
         WHERE directory_id = $1 AND category = $2 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(dir_id)
    .bind(REMINDER_CATEGORY)
    .fetch_optional(db)
    .await?
    {
        return Ok(t);
    }

    // Fall back to global template (no directory_id)
    if let Some(t) = sqlx::query_as::<_, EmailTemplate>(
        "SELECT id, name, subject, body, body_text, variables FROM email_templates \
         WHERE directory_id IS NULL AND category = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(REMINDER_CATEGORY)
    .fetch_optional(db)
    .await?
    {
        return Ok(t);
    }

    // Auto-create a branded default template
    let subject = "📊 Your {business_name} listing — check your stats".to_string();

    let p = &brand.primary;
    let b = &brand.background;
    let tc = &brand.text;

    let body = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"></head>
<body style="font-family:Inter,Arial,sans-serif;max-width:600px;margin:0 auto;padding:20px;background-color:{b};color:{tc};">
<table width="100%" cellpadding="0" cellspacing="0" style="background:{b};">
<tr><td style="padding:32px 24px;text-align:center;background:{p};">
  <h1 style="margin:0;color:#fff;font-size:24px;">Your {{{{directory_name}}}} Dashboard</h1>
</td></tr>
<tr><td style="padding:32px 24px;">
  <p style="font-size:16px;line-height:1.5;">Hi {{{{owner_name}}}},</p>
  <p style="font-size:16px;line-height:1.5;">
    It's been a while since you checked your
    <strong>{{{{business_name}}}}</strong> listing on <strong>{{{{directory_name}}}}</strong>.
    Here's what you might be missing:
  </p>
  <table width="100%" cellpadding="8" style="margin:24px 0;">
    <tr><td style="font-size:20px;width:40px;">📊</td><td>How many people are viewing your listing</td></tr>
    <tr><td style="font-size:20px;">📞</td><td>How many phone calls &amp; website clicks you're getting</td></tr>
    <tr><td style="font-size:20px;">📈</td><td>Weekly trends compared to last week</td></tr>
    <tr><td style="font-size:20px;">📍</td><td>Where your visitors are located</td></tr>
  </table>
  <p style="text-align:center;margin:32px 0;">
    <a href="{{{{dashboard_url}}}}"
       style="background:{p};color:#fff;padding:14px 32px;border-radius:8px;
              text-decoration:none;font-weight:bold;font-size:16px;display:inline-block;">
      View My Dashboard →
    </a>
  </p>
  <hr style="border:none;border-top:1px solid #e2e8f0;margin:32px 0;">
  <p style="color:#94a3b8;font-size:13px;line-height:1.4;">
    You're receiving this because you claimed a business listing on {{{{directory_name}}}}.
    If you'd rather not receive these reminders,
    <a href="{{{{dashboard_url}}}}?unsubscribe=reminders" style="color:{p};">unsubscribe here</a>.
  </p>
</td></tr>
<tr><td style="padding:16px 24px;text-align:center;background:{b};border-top:1px solid #e2e8f0;">
  <p style="color:#94a3b8;font-size:12px;margin:0;">{{{{directory_name}}}} — Powered by SwiftSoftware</p>
</td></tr></table>
</body></html>"#,
        p = p,
        b = b,
        tc = tc
    );

    let variables: Vec<String> = vec![
        "business_name".to_string(),
        "directory_name".to_string(),
        "owner_name".to_string(),
        "dashboard_url".to_string(),
        "primary_color".to_string(),
        "accent_color".to_string(),
        "background_color".to_string(),
        "text_color".to_string(),
    ];

    let created = sqlx::query_as::<_, EmailTemplate>(
        "INSERT INTO email_templates (name, subject, body, variables, category, directory_id) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, name, subject, body, body_text, variables",
    )
    .bind(format!("Dashboard Reminder — {dir_id}"))
    .bind(&subject)
    .bind(&body)
    .bind(&variables)
    .bind(REMINDER_CATEGORY)
    .bind(dir_id)
    .fetch_one(db)
    .await?;

    tracing::info!("[reminders] Auto-created branded template for directory {dir_id}");
    Ok(created)
}

/// Substitute {var} placeholders.
fn substitute(text: &str, vars: &HashMap<&str, String>) -> String {
    let mut result = text.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}

/// Look up the email signature for a directory.
async fn lookup_signature(db: &PgPool, dir_id: Uuid) -> (Option<String>, Option<String>) {
    sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT email_signature_html, email_signature_text FROM directories WHERE id = $1",
    )
    .bind(dir_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .unwrap_or((None, None))
}

/// Strip HTML tags for plain-text version.
fn strip_html(html: &str) -> String {
    let stripped = html
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n\n")
        .replace("</tr>", "\n")
        .replace("</td>", " ")
        .replace("</h1>", "\n\n")
        .replace("</h2>", "\n\n")
        .replace("</h3>", "\n\n");

    let mut cleaned = String::with_capacity(stripped.len());
    let mut in_tag = false;
    for ch in stripped.chars() {
        match ch {
            '<' if !in_tag => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => cleaned.push(ch),
            _ => {}
        }
    }
    cleaned
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn default_colors() -> BrandColors {
        BrandColors {
            primary: "#2563eb".into(),
            accent: "#f59e0b".into(),
            background: "#ffffff".into(),
            text: "#1e293b".into(),
        }
    }

    fn network_all(set: &str) -> NetworkBrandRow {
        NetworkBrandRow {
            primary_color: Some(format!("{set}11")),
            accent_color: Some(format!("{set}22")),
            background_color: Some(format!("{set}33")),
            text_color: Some(format!("{set}44")),
        }
    }

    /// A directory colour that IS set must win over the network's.
    #[test]
    fn directory_scheme_wins_over_network_branding() {
        let scheme = json!({ "primary": "#010101" });
        let net = network_all("#ff");
        let c = pick_colors(Some(&scheme), Some(&net), &default_colors());
        assert_eq!(c.primary, "#010101", "the directory's own colour must win");
        assert_eq!(c.accent, "#ff22", "the network must fill the gap");
        assert_eq!(c.background, "#ff33");
        assert_eq!(c.text, "#ff44");
    }

    /// The regression this module was fixed for: a network-branded directory with no
    /// usable `color_scheme` used to fall all the way through to the hardcoded default,
    /// so per-network branding never reached the reminder email.
    #[test]
    fn network_branding_is_used_when_the_directory_has_no_scheme() {
        let net = network_all("#aa");
        let c = pick_colors(None, Some(&net), &default_colors());
        assert_eq!(c.primary, "#aa11");
        assert_eq!(c.accent, "#aa22");
        assert_eq!(c.background, "#aa33");
        assert_eq!(c.text, "#aa44");
    }

    /// A network row that exists but leaves a colour NULL still falls back to the default
    /// for that one colour, and never drops the colours it does define.
    #[test]
    fn network_gaps_fall_through_to_the_default() {
        let net = NetworkBrandRow {
            primary_color: Some("#123456".into()),
            ..Default::default()
        };
        let c = pick_colors(None, Some(&net), &default_colors());
        assert_eq!(c.primary, "#123456");
        assert_eq!(c.accent, "#f59e0b");
        assert_eq!(c.background, "#ffffff");
        assert_eq!(c.text, "#1e293b");
    }

    /// No directory scheme and no network branding at all → the built-in palette.
    #[test]
    fn nothing_configured_uses_the_built_in_default() {
        let c = pick_colors(None, None, &default_colors());
        assert_eq!(c.primary, "#2563eb");
        assert_eq!(c.accent, "#f59e0b");
        assert_eq!(c.background, "#ffffff");
        assert_eq!(c.text, "#1e293b");
    }

    /// A non-string scheme value (e.g. a number) must not be rendered into the email;
    /// it falls through to the network/default exactly like an absent key.
    #[test]
    fn non_string_scheme_values_are_ignored() {
        let scheme = json!({ "primary": 12345 });
        let net = network_all("#55");
        let c = pick_colors(Some(&scheme), Some(&net), &default_colors());
        assert_eq!(c.primary, "#5511");
    }
}
