//! Stripe Connect onboarding for business payouts (card B143).
//!
//! The clearinghouse ([`crate::handlers::settlement`]) reimburses a redeeming business with a
//! Stripe transfer. Until now that transfer could only go to ONE network-level destination — the
//! operator's own account. To pay the BUSINESS on its own Stripe account the platform needs
//! Stripe Connect: the owner onboards through Stripe's hosted flow and the resulting connected
//! account id is stored per business (migration 161) so settlement can route the transfer to it.
//!
//! This module owns the onboarding surface:
//!
//! * `GET  /portal/business/payouts`            — the business's Connect state + whether Stripe is configured
//! * `POST /portal/business/payouts/connect`    — create/reuse an Express account, mint an onboarding link
//! * `POST /portal/business/payouts/refresh`    — re-read the account and record payouts_enabled
//! * `POST /portal/business/payouts/disconnect` — forget the connected account
//!
//! NOTHING HARDWIRED: the platform secret key and the API base both come from the active
//! `provider_keys` row for `provider='stripe'` (the key encrypted at rest as `enc:v1:`), so the
//! flow can be pointed at Stripe's sandbox or a local stub without a code change. When no active
//! key row exists every endpoint degrades honestly — a clear message, never a panic.

use axum::{extract::State, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::security::provider_key_crypto;
use crate::AppState;

/// Stripe's live API base. Overridable per provider row (`provider_keys.base_url`) so a sandbox
/// or a stub can be used without a rebuild.
const DEFAULT_STRIPE_BASE: &str = "https://api.stripe.com";

/// Where the browser should land when the hosted onboarding finishes without a caller-supplied
/// URL. The portal always sends its own origin, this is only a safety net.
const DEFAULT_RETURN_URL: &str = "https://zaarhub.com/portal";

struct StripePlatform {
    key: String,
    base: String,
    country: String,
}

/// The signed-in user's business, resolved the same way as the rest of the business portal:
/// the account's own claim first, then a legacy `user_id` claim.
async fn resolve_business_id(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Uuid> {
    if let Some(bid) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT business_id FROM claimed_businesses
           WHERE visitor_account_id = $1 AND business_id IS NOT NULL
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?
    {
        return Ok(bid);
    }

    if let Some(bid) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT business_id FROM claimed_businesses
           WHERE user_id = $1 AND business_id IS NOT NULL
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?
    {
        return Ok(bid);
    }

    Err(AppError::Forbidden(
        "No claimed business is linked to this account yet.".into(),
    ))
}

/// Resolve the platform's Stripe credential + API base, or `None` when Stripe is not configured.
async fn resolve_stripe_platform(db: &sqlx::PgPool) -> Option<StripePlatform> {
    let row = sqlx::query_as::<_, (String, Option<String>, Option<Value>)>(
        r#"SELECT api_key, base_url, metadata FROM provider_keys
           WHERE provider = 'stripe' AND is_active = true
           ORDER BY is_default DESC, updated_at DESC
           LIMIT 1"#,
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;

    let key = provider_key_crypto::decrypt_for_use(db, &row.0, "stripe").await?;
    if key.trim().is_empty() {
        return None;
    }

    let base = row
        .1
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_STRIPE_BASE.to_string());

    let country = row
        .2
        .as_ref()
        .and_then(|m| m.get("connect_country"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("US")
        .to_string();

    Some(StripePlatform { key, base, country })
}

/// One Stripe API call. Returns the parsed JSON body on 2xx and the provider's own error text
/// (never a fabricated success) on anything else.
async fn stripe_api(
    platform: &StripePlatform,
    method: reqwest::Method,
    path: &str,
    form: Option<Vec<(String, String)>>,
) -> ApiResult<Value> {
    let url = format!("{}{}", platform.base.trim_end_matches('/'), path);
    let client = reqwest::Client::new();
    let mut req = client.request(method, &url).bearer_auth(&platform.key);
    if let Some(f) = form {
        req = req.form(&f);
    }

    let res = req
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("stripe request failed: {e}")))?;

    let status = res.status();
    let body: Value = res.json().await.unwrap_or_else(|_| json!({}));

    if status.is_success() {
        Ok(body)
    } else {
        let msg = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("stripe returned HTTP {status}"));
        Err(AppError::Internal(format!("Stripe: {msg}")))
    }
}

/// Stripe's own account state → our closed status set.
fn status_from_account(acct: &Value) -> &'static str {
    let payouts = acct
        .get("payouts_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let details = acct
        .get("details_submitted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if payouts {
        "connected"
    } else if details {
        "restricted"
    } else {
        "pending"
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ConnectRow {
    stripe_connect_account_id: Option<String>,
    stripe_connect_status: String,
    stripe_payouts_enabled: bool,
}

async fn load_connect_row(db: &sqlx::PgPool, biz: Uuid) -> ApiResult<ConnectRow> {
    sqlx::query_as::<_, ConnectRow>(
        "SELECT stripe_connect_account_id, stripe_connect_status, stripe_payouts_enabled \
         FROM businesses WHERE id = $1",
    )
    .bind(biz)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::NotFound("business not found".into()))
}

/// Accept only absolute http(s) redirect URLs from the browser.
fn valid_redirect(u: &Option<String>) -> Option<String> {
    match u {
        Some(s) if s.starts_with("http://") || s.starts_with("https://") => Some(s.clone()),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
pub struct OnboardingRequest {
    #[serde(default)]
    pub return_url: Option<String>,
    #[serde(default)]
    pub refresh_url: Option<String>,
}

/// ── GET /api/v1/portal/business/payouts ──
/// The business's Connect state, plus whether the platform has a Stripe key at all.
pub async fn payouts_status(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let biz = resolve_business_id(&s.db, user_id).await?;
    let platform_configured = resolve_stripe_platform(&s.db).await.is_some();
    let row = load_connect_row(&s.db, biz).await?;

    Ok(Json(json!({
        "provider_configured": platform_configured,
        "business_id": biz,
        "account_id": row.stripe_connect_account_id,
        "status": row.stripe_connect_status,
        "payouts_enabled": row.stripe_payouts_enabled,
        "onboarding_available": platform_configured,
    })))
}

/// ── POST /api/v1/portal/business/payouts/connect ──
/// Create (or reuse) the business's Express account and return a hosted-onboarding URL.
pub async fn payouts_connect(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<OnboardingRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let biz = resolve_business_id(&s.db, user_id).await?;

    let Some(platform) = resolve_stripe_platform(&s.db).await else {
        return Err(AppError::BadRequest(
            "Stripe is not configured for this directory yet. Ask the operator to add the Stripe \
             key in the admin panel, then try again."
                .into(),
        ));
    };

    let (name, email): (String, Option<String>) =
        sqlx::query_as("SELECT name, email FROM businesses WHERE id = $1")
            .bind(biz)
            .fetch_one(&s.db)
            .await?;

    let row = load_connect_row(&s.db, biz).await?;

    let account_id = match row.stripe_connect_account_id.filter(|a| !a.is_empty()) {
        Some(existing) => existing,
        None => {
            let mut form: Vec<(String, String)> = vec![
                ("type".to_string(), "express".to_string()),
                ("country".to_string(), platform.country.clone()),
                ("business_profile[name]".to_string(), name.clone()),
                (
                    "capabilities[transfers][requested]".to_string(),
                    "true".to_string(),
                ),
            ];
            if let Some(e) = email.as_ref().filter(|e| !e.is_empty()) {
                form.push(("email".to_string(), e.clone()));
            }

            let acct =
                stripe_api(&platform, reqwest::Method::POST, "/v1/accounts", Some(form)).await?;
            let id = acct
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    AppError::Internal("Stripe did not return an account id".to_string())
                })?;

            sqlx::query(
                "UPDATE businesses SET stripe_connect_account_id = $2, \
                 stripe_connect_status = 'pending', stripe_connect_updated_at = NOW() \
                 WHERE id = $1",
            )
            .bind(biz)
            .bind(&id)
            .execute(&s.db)
            .await?;

            id
        }
    };

    let return_url =
        valid_redirect(&req.return_url).unwrap_or_else(|| DEFAULT_RETURN_URL.to_string());
    let refresh_url = valid_redirect(&req.refresh_url).unwrap_or_else(|| return_url.clone());

    let link = stripe_api(
        &platform,
        reqwest::Method::POST,
        "/v1/account_links",
        Some(vec![
            ("account".to_string(), account_id.clone()),
            ("type".to_string(), "account_onboarding".to_string()),
            ("return_url".to_string(), return_url),
            ("refresh_url".to_string(), refresh_url),
        ]),
    )
    .await?;

    let url = link
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            AppError::Internal("Stripe did not return an onboarding link".to_string())
        })?;

    Ok(Json(json!({
        "account_id": account_id,
        "status": "pending",
        "onboarding_url": url,
    })))
}

/// ── POST /api/v1/portal/business/payouts/refresh ──
/// Re-read the connected account from Stripe and record its real state.
pub async fn payouts_refresh(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let biz = resolve_business_id(&s.db, user_id).await?;
    let platform = resolve_stripe_platform(&s.db).await;

    let row = load_connect_row(&s.db, biz).await?;
    let Some(account_id) = row.stripe_connect_account_id.filter(|a| !a.is_empty()) else {
        return Err(AppError::BadRequest(
            "This business has not started Stripe onboarding yet.".into(),
        ));
    };
    let Some(platform) = platform else {
        return Err(AppError::BadRequest(
            "Stripe is not configured for this directory yet.".into(),
        ));
    };

    let acct = stripe_api(
        &platform,
        reqwest::Method::GET,
        &format!("/v1/accounts/{account_id}"),
        None,
    )
    .await?;

    let status = status_from_account(&acct);
    let payouts = acct
        .get("payouts_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    sqlx::query(
        "UPDATE businesses SET stripe_connect_status = $2, stripe_payouts_enabled = $3, \
         stripe_connect_updated_at = NOW() WHERE id = $1",
    )
    .bind(biz)
    .bind(status)
    .bind(payouts)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "account_id": account_id,
        "status": status,
        "payouts_enabled": payouts,
    })))
}

/// ── POST /api/v1/portal/business/payouts/disconnect ──
/// Forget the connected account on our side (Stripe's own account is left intact).
pub async fn payouts_disconnect(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let biz = resolve_business_id(&s.db, user_id).await?;

    sqlx::query(
        "UPDATE businesses SET stripe_connect_account_id = NULL, \
         stripe_connect_status = 'not_connected', stripe_payouts_enabled = false, \
         stripe_connect_updated_at = NOW() WHERE id = $1",
    )
    .bind(biz)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "status": "not_connected",
        "payouts_enabled": false,
    })))
}
