//! Payment provider management & checkout session creation.
//!
//! Endpoints:
//! - GET    /api/v1/payment-providers          (list configured providers, keys masked)
//! - POST   /api/v1/payment-providers          (create/update — super admin only)
//! - DELETE /api/v1/payment-providers/{type}   (remove — super admin only)
//! - POST   /api/v1/checkout/create            (create a Stripe/PayPal checkout session)
//! - GET    /api/v1/checkout/sessions          (list checkout sessions for this tenant)
//! - POST   /api/v1/webhooks/stripe            (Stripe webhook receiver — no auth)
//! - POST   /api/v1/webhooks/paypal            (PayPal webhook receiver — no auth)

use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use base64::{engine::general_purpose, Engine as _};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::{is_admin, is_super_admin};
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::security::provider_key_crypto as keycrypto;
use crate::AppState;

// ──────────────────────────────────────────────
// Admin: Payment Provider CRUD
// ──────────────────────────────────────────────

/// True when a stored (encrypted) credential column holds a value; `NULL` and `''` both mean
/// "not configured".
fn stored_column_has_value(v: &Option<String>) -> bool {
    v.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
}

/// Stripe and PayPal are the two gateways whose receivers verify a signature before an event may
/// complete a checkout, so each one needs a value stored before it may be armed:
///   * stripe — the endpoint's signing secret (`whsec_…`), used for the HMAC check
///   * paypal — the Webhook ID from the PayPal dashboard. PayPal verifies server-side through
///     `POST /v1/notifications/verify-webhook-signature`, which needs the Webhook ID plus the
///     client credentials; PayPal issues no HMAC secret, so the "webhook secret" field holds the id.
///
/// Activating either without that value used to be allowed, and the receiver then ACCEPTED
/// unverified events — a forged `checkout.session.completed` naming a pending
/// `provider_session_id` would have completed that session and fired fulfillment. The receivers now
/// fail closed; this guard stops the misconfiguration at the source so an operator cannot quietly
/// arm an unverified receiver.
fn require_webhook_config_for_activation(
    provider_type: &str,
    is_active: bool,
    api_key_present: bool,
    webhook_value_present: bool,
) -> Result<(), AppError> {
    if !is_active || !matches!(provider_type, "stripe" | "paypal") {
        return Ok(());
    }

    if !webhook_value_present {
        return Err(AppError::BadRequest(
            match provider_type {
                "stripe" => {
                    "Stripe cannot be activated without its webhook signing secret (whsec_…): \
                     without it /api/v1/webhooks/stripe cannot verify anything. Copy the signing \
                     secret from the webhook endpoint in the Stripe dashboard and save it in the \
                     same form."
                }
                _ => {
                    "PayPal cannot be activated without its Webhook ID: without it \
                     /api/v1/webhooks/paypal cannot verify anything. Copy the Webhook ID from the \
                     webhook in the PayPal dashboard and save it in the same form."
                }
            }
            .to_string(),
        ));
    }

    if provider_type == "paypal" && !api_key_present {
        return Err(AppError::BadRequest(
            "PayPal cannot be activated without its client_id:secret — PayPal verifies webhooks \
             server-side with those credentials."
                .into(),
        ));
    }

    Ok(())
}

/// GET /api/v1/payment-providers
/// List all configured payment providers (credentials never returned — status only)
pub async fn list_payment_providers(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(
        r#"SELECT id, provider_type, label, is_active,
                  CASE WHEN api_key_encrypted IS NOT NULL AND api_key_encrypted != '' THEN 'configured' ELSE 'not_configured' END as key_status,
                  CASE WHEN webhook_secret_encrypted IS NOT NULL AND webhook_secret_encrypted != '' THEN 'configured' ELSE 'not_configured' END as webhook_secret_status,
                  COALESCE(publishable_key, '') as publishable_key,
                  is_test_mode, config, created_at, updated_at
           FROM payment_providers
           ORDER BY provider_type ASC"#,
    )
    .fetch_all(&state.db)
    .await?;

    let providers: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid,_>("id").map(|u| u.to_string()).unwrap_or_default(),
                "provider_type": r.try_get::<&str,_>("provider_type").unwrap_or(""),
                "label": r.try_get::<&str,_>("label").unwrap_or(""),
                "is_active": r.try_get::<bool,_>("is_active").unwrap_or(false),
                "key_status": r.try_get::<&str,_>("key_status").unwrap_or("not_configured"),
                "webhook_secret_status": r.try_get::<&str,_>("webhook_secret_status").unwrap_or("not_configured"),
                "publishable_key": r.try_get::<&str,_>("publishable_key").unwrap_or(""),
                // The public receiver URL for this provider, so the admin panel can show the
                // operator exactly what to register in the gateway dashboard. Empty for the
                // provider types that have no receiver implemented.
                "webhook_url": match r.try_get::<&str,_>("provider_type").unwrap_or("") {
                    "stripe" => "/api/v1/webhooks/stripe",
                    "paypal" => "/api/v1/webhooks/paypal",
                    _ => "",
                },
                "is_test_mode": r.try_get::<bool,_>("is_test_mode").unwrap_or(true),
                "config": r.try_get::<serde_json::Value,_>("config").unwrap_or(json!({})),
                "created_at": r.try_get::<chrono::DateTime<chrono::Utc>,_>("created_at")
                    .map(|t| t.to_rfc3339()).unwrap_or_default(),
                "updated_at": r.try_get::<chrono::DateTime<chrono::Utc>,_>("updated_at")
                    .map(|t| t.to_rfc3339()).unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(json!({"providers": providers})))
}

/// POST /api/v1/payment-providers
/// Create or update a payment provider configuration
pub async fn upsert_payment_provider(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    // Super admin only
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Only super admins can manage payment providers".into(),
        ));
    }

    let provider_type = req
        .get("provider_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AppError::BadRequest(
                "provider_type is required (stripe, paypal, square, paddle)".into(),
            )
        })?;

    if !["stripe", "paypal", "square", "paddle"].contains(&provider_type) {
        return Err(AppError::BadRequest(
            "Invalid provider_type. Must be stripe, paypal, square, or paddle".into(),
        ));
    }

    let label = req.get("label").and_then(|v| v.as_str()).unwrap_or("");
    let is_active = req
        .get("is_active")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_test_mode = req
        .get("is_test_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let publishable_key = req
        .get("publishable_key")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let config = req.get("config").cloned().unwrap_or(json!({}));
    let api_key = req.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
    let webhook_secret = req
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Payment credentials are CUSTOMER-supplied secrets (a gateway secret key can move money, a
    // webhook secret authenticates payment events). Encrypt them BEFORE they reach the database
    // through the single choke point in src/security/provider_key_crypto.rs: enc:v1 + AES-256
    // under PROVIDER_KEY_ENC_SECRET, which lives only in the process environment. Fail-closed — a
    // missing master key errors here instead of storing the value the admin typed, and migration
    // 096's CHECK constraints refuse a plaintext write at the database. An empty value on the
    // update path means "keep the stored credential", so it is never re-encrypted.
    let stored_api_key = keycrypto::encrypt_for_storage(&state.db, api_key).await?;
    let stored_webhook_secret = keycrypto::encrypt_for_storage(&state.db, webhook_secret).await?;

    // Check if provider already exists. The stored credential columns come back too, so the
    // activation guard below sees the EFFECTIVE configuration — a blank field on an update means
    // "keep the stored credential", not "this provider has none".
    let existing = sqlx::query_as::<_, (Uuid, Option<String>, Option<String>)>(
        "SELECT id, api_key_encrypted, webhook_secret_encrypted FROM payment_providers \
         WHERE provider_type = $1",
    )
    .bind(provider_type)
    .fetch_optional(&state.db)
    .await?;

    if let Some((provider_id, stored_api_key_col, stored_webhook_secret_col)) = existing {
        // Refuse to ARM a receiver that has nothing to verify with — see
        // `require_webhook_config_for_activation` and the two webhook handlers below.
        require_webhook_config_for_activation(
            provider_type,
            is_active,
            !api_key.is_empty() || stored_column_has_value(&stored_api_key_col),
            !webhook_secret.is_empty() || stored_column_has_value(&stored_webhook_secret_col),
        )?;
        // Update — only overwrite api_key/webhook_secret if provided
        let mut query = String::from(
            "UPDATE payment_providers SET label = $1, is_active = $2, is_test_mode = $3, \
             publishable_key = $4, config = $5, updated_at = NOW()",
        );
        let mut param_idx = 6u8;

        if !api_key.is_empty() {
            query.push_str(&format!(", api_key_encrypted = ${}", param_idx));
            param_idx += 1;
        }
        if !webhook_secret.is_empty() {
            query.push_str(&format!(", webhook_secret_encrypted = ${}", param_idx));
            param_idx += 1;
        }
        query.push_str(&format!(" WHERE id = ${}", param_idx));

        let mut q = sqlx::query(&query)
            .bind(label)
            .bind(is_active)
            .bind(is_test_mode)
            .bind(publishable_key)
            .bind(&config);

        if !api_key.is_empty() {
            q = q.bind(&stored_api_key);
        }
        if !webhook_secret.is_empty() {
            q = q.bind(&stored_webhook_secret);
        }
        q = q.bind(provider_id);

        q.execute(&state.db).await?;

        Ok(Json(json!({
            "status": "updated",
            "provider_type": provider_type,
            "message": "Payment provider updated"
        })))
    } else {
        // Insert
        if api_key.is_empty() {
            return Err(AppError::BadRequest(
                "api_key is required when creating a new provider".into(),
            ));
        }

        // Same guard as the update path: a receiver may not be armed without its verification value.
        require_webhook_config_for_activation(
            provider_type,
            is_active,
            !api_key.is_empty(),
            !webhook_secret.is_empty(),
        )?;

        sqlx::query(
            r#"INSERT INTO payment_providers
               (provider_type, label, is_active, api_key_encrypted, webhook_secret_encrypted,
                publishable_key, config, is_test_mode)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(provider_type)
        .bind(label)
        .bind(is_active)
        .bind(&stored_api_key)
        .bind(&stored_webhook_secret)
        .bind(publishable_key)
        .bind(&config)
        .bind(is_test_mode)
        .execute(&state.db)
        .await?;

        Ok(Json(json!({
            "status": "created",
            "provider_type": provider_type,
            "message": "Payment provider created"
        })))
    }
}

/// DELETE /api/v1/payment-providers/{provider_type}
/// Remove a payment provider configuration
pub async fn delete_payment_provider(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(provider_type): Path<String>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Only super admins can manage payment providers".into(),
        ));
    }

    let result = sqlx::query("DELETE FROM payment_providers WHERE provider_type = $1")
        .bind(&provider_type)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Payment provider '{}' not found",
            provider_type
        )));
    }

    Ok(Json(
        json!({"status": "deleted", "provider_type": provider_type}),
    ))
}

// ──────────────────────────────────────────────
// Checkout Session Creation
// ──────────────────────────────────────────────

/// Decrypted credentials of the active provider for one payment type.
///
/// Payment credentials are stored as `enc:v1:` ciphertext under the env-only master key (see
/// [`crate::security::provider_key_crypto`]). This struct is the ONLY shape in which they leave the
/// database, and it is deliberately not `Serialize`: this lookup used to return a
/// `serde_json::Value` that carried the raw stored column under the key `"api_key"`, so any caller
/// that echoed that value into a response would have handed the credential (or its ciphertext) to a
/// client. No field here holds the stored value.
struct ActiveProvider {
    /// Decrypted, for use. Never log directly — use [`ActiveProvider::api_key_mask`].
    api_key: String,
    /// Decrypted, for use (Stripe/PayPal webhook signature verification). For PayPal this holds the
    /// operator's PayPal Webhook ID — see `verify_paypal_webhook`.
    webhook_secret: String,
    /// PayPal runs a sandbox and a live environment on different hosts, and a webhook signature made
    /// in one is not valid in the other: the receiver must verify against the host the provider is
    /// configured for.
    is_test_mode: bool,
}

impl ActiveProvider {
    /// A `sk-l...2345` mask derived from the DECRYPTED key — safe for logs and diagnostics.
    fn api_key_mask(&self) -> String {
        super::provider_keys_handler::mask_key(&self.api_key)
    }
}

/// Get active payment provider configuration, credentials DECRYPTED for use.
async fn get_active_provider(
    db: &sqlx::PgPool,
    provider_type: &str,
) -> Result<Option<ActiveProvider>, AppError> {
    let row = sqlx::query(
        r#"SELECT api_key_encrypted, webhook_secret_encrypted, is_test_mode
           FROM payment_providers
           WHERE provider_type = $1 AND is_active = true
           LIMIT 1"#,
    )
    .bind(provider_type)
    .fetch_optional(db)
    .await?;

    let Some(r) = row else {
        return Ok(None);
    };

    // Read-for-USE: the columns hold ciphertext, so decrypt here — once — and never hand the
    // stored value to a caller. A value that cannot be decrypted (missing/rotated master key)
    // degrades to "no usable credential" (log + skip) instead of putting ciphertext on the wire to
    // Stripe/PayPal as if it were a credential.
    let stored_api_key: Option<String> = r.try_get("api_key_encrypted")?;
    let stored_webhook_secret: Option<String> = r.try_get("webhook_secret_encrypted")?;

    let api_key = match stored_api_key.filter(|v| !v.is_empty()) {
        Some(stored) => keycrypto::decrypt_for_use(db, &stored, provider_type)
            .await
            .unwrap_or_default(),
        None => String::new(),
    };
    let webhook_secret = match stored_webhook_secret.filter(|v| !v.is_empty()) {
        Some(stored) => keycrypto::decrypt_for_use(db, &stored, provider_type)
            .await
            .unwrap_or_default(),
        None => String::new(),
    };

    Ok(Some(ActiveProvider {
        api_key,
        webhook_secret,
        // An unknown value must not silently mean "sandbox": default to the live host.
        is_test_mode: r.try_get::<bool, _>("is_test_mode").unwrap_or(false),
    }))
}

/// POST /api/v1/checkout/create
/// Create a Stripe/PayPal checkout session
pub async fn create_checkout_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let provider_type = req
        .get("provider_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("provider_type is required (stripe, paypal)".into()))?;

    let purchasable_type = req
        .get("purchasable_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("purchasable_type is required".into()))?;

    let amount = req
        .get("amount")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| AppError::BadRequest("amount is required".into()))?;

    let currency = req
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("USD");
    let purchasable_id = req
        .get("purchasable_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let success_url = req
        .get("success_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("success_url is required".into()))?;

    let cancel_url = req
        .get("cancel_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("cancel_url is required".into()))?;

    let metadata = req.get("metadata").cloned().unwrap_or(json!({}));

    // Get the active provider config
    let provider = get_active_provider(&state.db, provider_type)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!("No active {} provider configured", provider_type))
        })?;

    let api_key = provider.api_key.clone();
    if api_key.is_empty() {
        return Err(AppError::BadRequest(format!(
            "{} API key not configured",
            provider_type
        )));
    }

    // Log the posture from the DECRYPTED value's mask — the mask is what is safe to keep, the
    // credential itself never reaches a log line.
    tracing::info!(
        provider = provider_type,
        api_key = %provider.api_key_mask(),
        "creating checkout session with the decrypted gateway credential"
    );

    // Create checkout session with the provider
    let provider_session = match provider_type {
        "stripe" => {
            create_stripe_session(
                &api_key,
                amount,
                currency,
                purchasable_type,
                success_url,
                cancel_url,
                &metadata,
            )
            .await?
        }
        "paypal" => {
            create_paypal_session(
                &api_key,
                paypal_api_base(provider.is_test_mode),
                amount,
                currency,
                purchasable_type,
                success_url,
                cancel_url,
                &metadata,
            )
            .await?
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "Checkout not supported for provider type: {}",
                provider_type
            )))
        }
    };

    let provider_session_id = provider_session["id"].as_str().unwrap_or("");
    let checkout_url = provider_session["url"].as_str().unwrap_or("");

    // Store the checkout session in our database.
    //
    // checkout_sessions keys a session to the BUYING BUSINESS (business_id is NOT NULL and a
    // FK to businesses) plus its directory. The original code bound account_id/user_id —
    // neither column exists — so every checkout creation 500'd (kanban t_5356f3fb).
    let session_id = Uuid::new_v4();
    let (business_id, business_directory_id) =
        resolve_checkout_business(&state.db, user_id, tenant_id).await?;
    sqlx::query(
        r#"INSERT INTO checkout_sessions
           (id, business_id, directory_id, provider_type, provider_session_id,
            purchasable_type, purchasable_id, amount, currency, status, metadata)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending', $10)"#,
    )
    .bind(session_id)
    .bind(business_id)
    .bind(business_directory_id)
    .bind(provider_type)
    .bind(provider_session_id)
    .bind(purchasable_type)
    .bind(purchasable_id)
    .bind(amount)
    .bind(currency)
    .bind(&metadata)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "session_id": session_id.to_string(),
        "provider_session_id": provider_session_id,
        "checkout_url": checkout_url,
        "provider_type": provider_type,
    })))
}

/// Create a Stripe checkout session via Stripe API
async fn create_stripe_session(
    api_key: &str,
    amount: f64,
    currency: &str,
    purchasable_type: &str,
    success_url: &str,
    cancel_url: &str,
    metadata: &serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let client = reqwest::Client::new();

    // Stripe expects amount in cents
    let amount_cents = (amount * 100.0).round() as u64;

    // Build the line item
    let mut line_item = serde_json::json!({
        "price_data": {
            "currency": currency.to_lowercase(),
            "product_data": {
                "name": format!("{} purchase", purchasable_type.replace('_', " ")),
            },
            "unit_amount": amount_cents,
        },
        "quantity": 1,
    });

    // Add description from metadata if present
    if let Some(desc) = metadata.get("description").and_then(|v| v.as_str()) {
        line_item["price_data"]["product_data"]["description"] = json!(desc);
    }

    let mut body = serde_json::json!({
        "mode": "payment",
        "success_url": success_url,
        "cancel_url": cancel_url,
        "line_items": [line_item],
        "metadata": metadata.clone(),
    });

    // Map metadata to Stripe's flat format — all values must be strings
    if let Some(obj) = body["metadata"].as_object_mut() {
        for (_k, v) in obj.iter_mut() {
            if !v.is_string() {
                *v = json!(v.to_string());
            }
        }
    }

    let resp = client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&to_stripe_form_data(&body))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Stripe API error: {}", e)))?;

    let status = resp.status();
    let response_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse Stripe response: {}", e)))?;

    if !status.is_success() {
        let error_msg = response_body["error"]["message"]
            .as_str()
            .unwrap_or("Unknown Stripe error");
        return Err(AppError::Internal(format!("Stripe error: {}", error_msg)));
    }

    Ok(json!({
        "id": response_body["id"].as_str().unwrap_or(""),
        "url": response_body["url"].as_str().unwrap_or(""),
    }))
}

/// Create a PayPal order via PayPal REST API
async fn create_paypal_session(
    api_key: &str,
    api_base: &str,
    amount: f64,
    currency: &str,
    _purchasable_type: &str,
    success_url: &str,
    cancel_url: &str,
    _metadata: &serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let client = reqwest::Client::new();

    // PayPal requires an access token first
    let token_resp = client
        .post(format!("{}/v1/oauth2/token", api_base))
        .header(
            "Authorization",
            format!("Basic {}", base64_encode_auth(api_key)),
        )
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("PayPal auth error: {}", e)))?;

    let token_body: serde_json::Value = token_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse PayPal auth response: {}", e)))?;

    let access_token = token_body["access_token"]
        .as_str()
        .ok_or_else(|| AppError::Internal("Failed to get PayPal access token".into()))?;

    // Create the order
    let order_body = serde_json::json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "amount": {
                "currency_code": currency.to_uppercase(),
                "value": format!("{:.2}", amount),
            }
        }],
        "payment_source": {
            "paypal": {
                "experience_context": {
                    "payment_method_preference": "IMMEDIATE_PAYMENT_REQUIRED",
                    "landing_page": "LOGIN",
                    "user_action": "PAY_NOW",
                    "return_url": success_url,
                    "cancel_url": cancel_url,
                }
            }
        }
    });

    let order_resp = client
        .post(format!("{}/v2/checkout/orders", api_base))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("PayPal-Request-Id", format!("order-{}", Uuid::new_v4()))
        .json(&order_body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("PayPal order error: {}", e)))?;

    let order_status = order_resp.status();
    let order_body: serde_json::Value = order_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse PayPal order response: {}", e)))?;

    if !order_status.is_success() {
        let error_msg = order_body["message"]
            .as_str()
            .or_else(|| order_body["error_description"].as_str())
            .unwrap_or("Unknown PayPal error");
        return Err(AppError::Internal(format!("PayPal error: {}", error_msg)));
    }

    // Get the approval URL from the links
    let approval_url = order_body["links"]
        .as_array()
        .and_then(|links| {
            links
                .iter()
                .find(|l| l["rel"].as_str() == Some("approve"))
                .and_then(|l| l["href"].as_str())
        })
        .unwrap_or("");

    Ok(json!({
        "id": order_body["id"].as_str().unwrap_or(""),
        "url": approval_url,
    }))
}

// ──────────────────────────────────────────────
// Webhook Handlers (public — no auth)
// ──────────────────────────────────────────────

/// POST /api/v1/webhooks/stripe
/// Handle incoming Stripe webhook events
pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<impl IntoResponse> {
    let event_body: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("Invalid JSON: {}", e)))?;

    let event_type = event_body["type"].as_str().unwrap_or("unknown");
    let event_id = event_body["id"].as_str().unwrap_or("");

    // Get the active Stripe provider for webhook secret verification
    let provider = get_active_provider(&state.db, "stripe").await?;

    // Extract the signature header for verification
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // FAIL CLOSED. This receiver used to answer "accept" whenever no webhook_secret was stored, so
    // an anonymous POST could complete a pending checkout session for any gateway configured
    // without one. Every path that is not a verified signature now REJECTS: no active provider, no
    // stored secret, no signature header, a wrong signature, or a signature outside the timestamp
    // tolerance (replay).
    let rejection: Option<&'static str> = match provider.as_ref() {
        None => Some("no_active_stripe_provider_configured"),
        Some(prov) if prov.webhook_secret.is_empty() => Some("no_webhook_secret_configured"),
        Some(_) if signature.is_empty() => Some("missing_signature_header"),
        Some(prov) if !verify_stripe_signature(&body, signature, &prov.webhook_secret) => {
            Some("signature_verification_failed")
        }
        Some(_) => None,
    };

    if let Some(reason) = rejection {
        match reason {
            // A misconfiguration the operator can fix from the panel: say so loudly.
            "no_webhook_secret_configured" => tracing::error!(
                provider = "stripe",
                event_id,
                "Stripe webhook REJECTED — the active Stripe provider has no signing secret stored, \
                 so no event can be verified. Add the endpoint's whsec_… in Admin > Payment gateways."
            ),
            _ => tracing::warn!(provider = "stripe", event_id, reason, "Stripe webhook rejected"),
        }
    }

    // Log the event either way — a refusal is evidence and belongs in payment_webhook_events.
    let db_status = if rejection.is_none() {
        "received"
    } else {
        "failed"
    };
    sqlx::query(
        r#"INSERT INTO payment_webhook_events
           (provider_type, event_type, event_id, raw_body, headers, status, error_message)
           VALUES ('stripe', $1, $2, $3, $4, $5, $6)"#,
    )
    .bind(event_type)
    .bind(event_id)
    .bind(&event_body)
    .bind(&json!({"stripe-signature": signature}))
    .bind(db_status)
    .bind(rejection)
    .execute(&state.db)
    .await?;

    if let Some(reason) = rejection {
        return Ok((
            StatusCode::OK,
            Json(json!({"status": "ignored", "reason": reason})),
        ));
    }

    // Handle the event
    match event_type {
        "checkout.session.completed" => {
            handle_checkout_completed(&state.db, &event_body, "stripe").await?;
        }
        "checkout.session.expired" => {
            if let Some(session) = event_body.get("data").and_then(|d| d.get("object")) {
                let provider_session_id = session["id"].as_str().unwrap_or("");
                mark_session_expired(&state.db, "stripe", provider_session_id).await?;
            }
        }
        _ => {
            sqlx::query("UPDATE payment_webhook_events SET status = 'ignored' WHERE event_id = $1")
                .bind(event_id)
                .execute(&state.db)
                .await?;
        }
    }

    Ok((StatusCode::OK, Json(json!({"status": "processed"}))))
}

/// POST /api/v1/webhooks/paypal
/// Handle incoming PayPal webhook events
pub async fn paypal_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<impl IntoResponse> {
    let event_body: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("Invalid JSON: {}", e)))?;

    let event_type = event_body["event_type"].as_str().unwrap_or("unknown");
    let event_id = event_body["id"].as_str().unwrap_or("");

    // The five transmission headers PayPal signs with. This receiver used to log ONE of them and
    // act on the body regardless, so any caller could post PAYMENT.CAPTURE.COMPLETED and the
    // checkout session was marked completed. Verification is now the supported server-side check,
    // and anything unverified is refused.
    let hdr = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    };
    let transmission_id = hdr("paypal-transmission-id");
    let transmission_time = hdr("paypal-transmission-time");
    let cert_url = hdr("paypal-cert-url");
    let auth_algo = hdr("paypal-auth-algo");
    let transmission_sig = hdr("paypal-transmission-sig");

    let provider = get_active_provider(&state.db, "paypal").await?;

    let rejection: Option<&'static str> = match provider.as_ref() {
        None => Some("no_active_paypal_provider_configured"),
        Some(prov) if prov.webhook_secret.is_empty() => Some("no_webhook_id_configured"),
        Some(_)
            if transmission_id.is_empty()
                || transmission_time.is_empty()
                || cert_url.is_empty()
                || auth_algo.is_empty()
                || transmission_sig.is_empty() =>
        {
            Some("missing_transmission_headers")
        }
        Some(prov) => {
            match verify_paypal_webhook(
                prov,
                &event_body,
                transmission_id,
                transmission_time,
                cert_url,
                auth_algo,
                transmission_sig,
            )
            .await
            {
                Ok(true) => None,
                Ok(false) => Some("signature_verification_failed"),
                // PayPal could not be asked (network, bad client credentials, unknown webhook id).
                // An event that cannot be verified is an event that is not acted on.
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        provider = "paypal",
                        event_id,
                        "PayPal webhook verification could not be completed — rejecting"
                    );
                    Some("verification_unavailable")
                }
            }
        }
    };

    if let Some(reason) = rejection {
        if reason == "no_webhook_id_configured" {
            tracing::error!(
                provider = "paypal",
                event_id,
                "PayPal webhook REJECTED — the active PayPal provider has no Webhook ID stored, so \
                 no event can be verified. Add it in Admin > Payment gateways."
            );
        } else {
            tracing::warn!(
                provider = "paypal",
                event_id,
                reason,
                "PayPal webhook rejected"
            );
        }
    }

    // Log the event either way — a refusal is evidence and belongs in payment_webhook_events.
    let hdrs = json!({
        "paypal-transmission-id": transmission_id,
        "paypal-transmission-time": transmission_time,
        "paypal-transmission-sig": transmission_sig,
        "paypal-cert-url": cert_url,
        "paypal-auth-algo": auth_algo,
    });
    let db_status = if rejection.is_none() {
        "received"
    } else {
        "failed"
    };
    sqlx::query(
        r#"INSERT INTO payment_webhook_events
           (provider_type, event_type, event_id, raw_body, headers, status, error_message)
           VALUES ('paypal', $1, $2, $3, $4, $5, $6)"#,
    )
    .bind(event_type)
    .bind(event_id)
    .bind(&event_body)
    .bind(&hdrs)
    .bind(db_status)
    .bind(rejection)
    .execute(&state.db)
    .await?;

    if let Some(reason) = rejection {
        return Ok((
            StatusCode::OK,
            Json(json!({"status": "ignored", "reason": reason})),
        ));
    }

    match event_type {
        "CHECKOUT.ORDER.APPROVED" | "PAYMENT.CAPTURE.COMPLETED" => {
            handle_checkout_completed(&state.db, &event_body, "paypal").await?;
        }
        _ => {
            sqlx::query("UPDATE payment_webhook_events SET status = 'ignored' WHERE event_id = $1")
                .bind(event_id)
                .execute(&state.db)
                .await?;
        }
    }

    Ok((StatusCode::OK, Json(json!({"status": "processed"}))))
}

// ──────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────

/// Handle a completed checkout — update session status and trigger fulfillment
async fn handle_checkout_completed(
    db: &sqlx::PgPool,
    event_body: &serde_json::Value,
    provider_type: &str,
) -> Result<(), AppError> {
    let session = match provider_type {
        "stripe" => event_body["data"]["object"].clone(),
        "paypal" => event_body["resource"].clone(),
        _ => return Ok(()),
    };

    let provider_session_id = match provider_type {
        "stripe" => session["id"].as_str().map(|s| s.to_string()),
        "paypal" => session["id"].as_str().map(|s| s.to_string()),
        _ => None,
    };

    if provider_session_id.is_none() {
        tracing::warn!("Webhook received without provider session ID");
        return Ok(());
    }

    let provider_session_id = provider_session_id.unwrap();

    // Update the checkout session status
    let result = sqlx::query(
        r#"UPDATE checkout_sessions
           SET status = 'completed',
               webhook_received_at = NOW(),
               webhook_event_id = $1,
               updated_at = NOW()
           WHERE provider_session_id = $2
             AND provider_type = $3
             AND status = 'pending'"#,
    )
    .bind(event_body["id"].as_str().unwrap_or(""))
    .bind(&provider_session_id)
    .bind(provider_type)
    .execute(db)
    .await?;

    if result.rows_affected() == 0 {
        tracing::warn!(
            "No pending checkout session found for provider session: {}",
            provider_session_id
        );
        return Ok(());
    }

    // Mark the webhook event as processed
    sqlx::query("UPDATE payment_webhook_events SET status = 'processed' WHERE event_id = $1")
        .bind(event_body["id"].as_str().unwrap_or(""))
        .execute(db)
        .await?;

    tracing::info!(
        "Checkout completed: provider_session={}",
        provider_session_id
    );
    Ok(())
}

/// Mark a checkout session as expired
async fn mark_session_expired(
    db: &sqlx::PgPool,
    provider_type: &str,
    provider_session_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE checkout_sessions SET status = 'expired', updated_at = NOW() \
         WHERE provider_session_id = $1 AND provider_type = $2 AND status = 'pending'",
    )
    .bind(provider_session_id)
    .bind(provider_type)
    .execute(db)
    .await?;

    Ok(())
}

// ──────────────────────────────────────────────
// GET /api/v1/checkout/sessions
// List checkout sessions for the authenticated tenant
// ──────────────────────────────────────────────

pub async fn list_checkout_sessions(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // A session belongs to the buying business. account_id was never a column on
    // checkout_sessions, so this route 500'd for everyone (kanban t_5356f3fb).
    // A platform admin with no claimed business still sees the whole book.
    let scope = match resolve_checkout_business(&state.db, user_id, tenant_id).await {
        Ok((business_id, _dir)) => Ok(Some(business_id)),
        Err(e) => {
            if is_admin(&claims) {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }?;

    let rows = match scope {
        Some(business_id) => {
            sqlx::query(
                r#"SELECT id, business_id, directory_id, provider_type, purchasable_type,
                          purchasable_id::text, amount::text, currency, status,
                          provider_session_id, created_at, updated_at
                   FROM checkout_sessions
                   WHERE business_id = $1
                   ORDER BY created_at DESC
                   LIMIT 50"#,
            )
            .bind(business_id)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query(
                r#"SELECT id, business_id, directory_id, provider_type, purchasable_type,
                          purchasable_id::text, amount::text, currency, status,
                          provider_session_id, created_at, updated_at
                   FROM checkout_sessions
                   ORDER BY created_at DESC
                   LIMIT 50"#,
            )
            .fetch_all(&state.db)
            .await?
        }
    };

    let sessions: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid, _>("id").map(|u| u.to_string()).unwrap_or_default(),
                "business_id": r.try_get::<Option<Uuid>, _>("business_id").map(|u| u.map(|x| x.to_string())).unwrap_or(None),
                "directory_id": r.try_get::<Option<Uuid>, _>("directory_id").map(|u| u.map(|x| x.to_string())).unwrap_or(None),
                "provider_type": r.try_get::<&str, _>("provider_type").unwrap_or(""),
                "purchasable_type": r.try_get::<&str, _>("purchasable_type").unwrap_or(""),
                "purchasable_id": r.try_get::<Option<&str>, _>("purchasable_id").unwrap_or(None),
                "amount": r.try_get::<&str, _>("amount").unwrap_or("0"),
                "currency": r.try_get::<&str, _>("currency").unwrap_or(""),
                "status": r.try_get::<&str, _>("status").unwrap_or(""),
                "provider_session_id": r.try_get::<Option<&str>, _>("provider_session_id").unwrap_or(None),
                "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .map(|t| t.to_rfc3339()).unwrap_or_default(),
                "updated_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
                    .map(|t| t.to_rfc3339()).unwrap_or_default(),
            })
        })
        .collect();

    Ok(Json(json!({"sessions": sessions, "total": sessions.len()})))
}

/// Resolve the business a checkout session belongs to, plus its directory.
///
/// Order: the claimed business of the authenticated user (any business type), then — for
/// tokens minted before a claim existed — the tenant id itself being a business id.
/// Returns a clear 400 instead of a 500 when the account owns no business at all.
async fn resolve_checkout_business(
    db: &sqlx::PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
) -> ApiResult<(Uuid, Option<Uuid>)> {
    if let Ok(biz) = crate::handlers::b2b::resolve_buyer_business(db, user_id).await {
        let dir = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT directory_id FROM businesses WHERE id = $1",
        )
        .bind(biz)
        .fetch_optional(db)
        .await?
        .flatten();
        return Ok((biz, dir));
    }

    let row = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT id, directory_id FROM businesses WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(db)
    .await?;

    row.ok_or_else(|| {
        AppError::BadRequest(
            "No business is linked to this account — claim a business before checking out".into(),
        )
    })
}

/// PayPal runs a sandbox and a live environment on different hosts. A webhook signed by one is not
/// valid in the other, so both the checkout call and the webhook verification use the host the
/// provider row is configured for (`is_test_mode`). Before this, the toggle was stored and shown in
/// the panel but ignored, so a sandbox-configured PayPal always talked to live.
fn paypal_api_base(is_test_mode: bool) -> &'static str {
    if is_test_mode {
        "https://api-m.sandbox.paypal.com"
    } else {
        "https://api-m.paypal.com"
    }
}

/// Verify a PayPal webhook the way PayPal supports it: server-side, by asking PayPal to validate the
/// transmission signature (`POST /v1/notifications/verify-webhook-signature`). There is no HMAC
/// secret to compare locally — `provider.webhook_secret` holds the operator's PayPal **Webhook ID**
/// and `provider.api_key` holds the `client_id:secret` pair the call authenticates with.
///
/// Returns `Ok(true)` only for PayPal's own `verification_status: SUCCESS`. Every other outcome —
/// a FAILURE, or an error from the call itself — must be treated as "not verified" by the caller.
async fn verify_paypal_webhook(
    provider: &ActiveProvider,
    event_body: &serde_json::Value,
    transmission_id: &str,
    transmission_time: &str,
    cert_url: &str,
    auth_algo: &str,
    transmission_sig: &str,
) -> Result<bool, AppError> {
    let client = reqwest::Client::new();
    let api_base = paypal_api_base(provider.is_test_mode);

    let token_resp = client
        .post(format!("{}/v1/oauth2/token", api_base))
        .header(
            "Authorization",
            format!("Basic {}", base64_encode_auth(&provider.api_key)),
        )
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("PayPal auth error: {}", e)))?;

    let token_body: serde_json::Value = token_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse PayPal auth response: {}", e)))?;

    let access_token = token_body["access_token"].as_str().ok_or_else(|| {
        AppError::Internal("Failed to get a PayPal access token for webhook verification".into())
    })?;

    let payload = json!({
        "transmission_id": transmission_id,
        "transmission_time": transmission_time,
        "cert_url": cert_url,
        "auth_algo": auth_algo,
        "transmission_sig": transmission_sig,
        "webhook_id": provider.webhook_secret,
        "webhook_event": event_body,
    });

    let resp = client
        .post(format!(
            "{}/v1/notifications/verify-webhook-signature",
            api_base
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("PayPal verify-webhook-signature error: {}", e)))?;

    let http_status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| json!({}));

    if !http_status.is_success() {
        // PayPal answers 4xx when it cannot even run the check (unknown Webhook ID, rejected client
        // credentials). That is NOT a pass: refuse the event and name the reason.
        return Err(AppError::Internal(format!(
            "PayPal verify-webhook-signature returned HTTP {}: {}",
            http_status,
            body["message"]
                .as_str()
                .or_else(|| body["error_description"].as_str())
                .unwrap_or("no detail")
        )));
    }

    Ok(body["verification_status"].as_str() == Some("SUCCESS"))
}

/// Verify a Stripe webhook signature: HMAC-SHA256 over `"{timestamp}.{body}"`, compared in constant
/// time, with the timestamp inside a replay window.
///
/// - constant time: `ring::hmac::verify` is used instead of comparing two hex strings with `==`
///   (which leaks how much of a guessed signature matched).
/// - replay window: an event whose signed timestamp is more than `TOLERANCE_SECS` away from now is
///   refused, per Stripe's guidance, so a captured signed body cannot be replayed later.
/// - the payload is built from raw bytes, not from a lossy UTF-8 conversion, so a body containing
///   invalid UTF-8 can never be mangled into something that verifies.
fn verify_stripe_signature(body: &[u8], signature: &str, secret: &str) -> bool {
    use ring::hmac;

    /// Stripe's documented default tolerance for the signed timestamp.
    const TOLERANCE_SECS: i64 = 300;

    // Stripe sends signatures in the format: t=timestamp,v1=signature
    let mut timestamp = "";
    let mut expected_sig = "";

    for part in signature.split(',') {
        if let Some(t) = part.strip_prefix("t=") {
            timestamp = t.trim();
        } else if let Some(s) = part.strip_prefix("v1=") {
            if expected_sig.is_empty() {
                expected_sig = s.trim();
            }
        }
    }

    if timestamp.is_empty() || expected_sig.is_empty() {
        return false;
    }

    let Ok(ts) = timestamp.parse::<i64>() else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if (now - ts).abs() > TOLERANCE_SECS {
        tracing::warn!(
            skew_secs = now - ts,
            "Stripe webhook signature timestamp is outside the replay tolerance — rejected"
        );
        return false;
    }

    let Ok(expected) = hex::decode(expected_sig) else {
        return false;
    };

    // The payload is `timestamp + "." + body`, byte for byte as Stripe signed it.
    let mut payload = Vec::with_capacity(timestamp.len() + 1 + body.len());
    payload.extend_from_slice(timestamp.as_bytes());
    payload.push(b'.');
    payload.extend_from_slice(body);

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hmac::verify(&key, &payload, &expected).is_ok()
}

/// Convert a JSON value to URL-encoded form data for Stripe API
fn to_stripe_form_data(value: &serde_json::Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();

    fn flatten(prefix: &str, value: &serde_json::Value, pairs: &mut Vec<(String, String)>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}[{}]", prefix, k)
                    };
                    flatten(&key, v, pairs);
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    let key = format!("{}[{}]", prefix, i);
                    flatten(&key, v, pairs);
                }
            }
            serde_json::Value::String(s) => {
                pairs.push((prefix.to_string(), s.clone()));
            }
            serde_json::Value::Number(n) => {
                pairs.push((prefix.to_string(), n.to_string()));
            }
            serde_json::Value::Bool(b) => {
                pairs.push((prefix.to_string(), b.to_string()));
            }
            serde_json::Value::Null => {
                pairs.push((prefix.to_string(), String::new()));
            }
        }
    }

    flatten("", value, &mut pairs);
    pairs
}

/// Base64-encode a client_id:secret pair for PayPal Basic auth
fn base64_encode_auth(credentials: &str) -> String {
    general_purpose::STANDARD.encode(credentials.as_bytes())
}

/// GET /api/v1/checkout/session/:id — public payment-confirmation lookup by session id
/// (accepts either the internal uuid or the provider session id). Round 9: thank-you.html
/// called this and got the SPA HTML fallback, so the confirmation page never showed details.
pub async fn get_checkout_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    #[allow(clippy::type_complexity)]
    let row = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            Option<Uuid>,
            Option<Uuid>,
            Option<serde_json::Value>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"SELECT status, purchasable_type, provider_type, business_id, directory_id,
                  metadata, amount::text, currency
           FROM checkout_sessions
           WHERE id::text = $1 OR provider_session_id = $1
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Checkout session not found".to_string()))?;

    let (
        status,
        purchasable_type,
        provider_type,
        business_id,
        directory_id,
        metadata,
        amount,
        currency,
    ) = row;

    let plan_name = metadata
        .as_ref()
        .and_then(|m| m.get("plan_name"))
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .unwrap_or_else(|| purchasable_type.clone());

    Ok(Json(json!({
        "session_id": id,
        "status": status,
        "purchasable_type": purchasable_type,
        "provider_type": provider_type,
        "plan_name": plan_name,
        "business_id": business_id,
        "directory_id": directory_id,
        "amount": amount,
        "currency": currency,
        "login_url": "/admin",
    })))
}
