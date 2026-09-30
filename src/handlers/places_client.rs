//! Google Places (LEGACY) response handling — the answer, never swallowed.
//!
//! The Maps **Places** API answers HTTP 200 even when it REFUSED the request: the
//! refusal lives in the JSON body's `status` field (`REQUEST_DENIED`,
//! `OVER_QUERY_LIMIT`, `INVALID_REQUEST`, `UNKNOWN_ERROR` …) with a human
//! `error_message`. Treating that body like a successful "no results" response —
//! the bug behind card B78 — makes a rejected key indistinguishable from a genuine
//! miss, so a wrong/expired key looked exactly like an empty directory for weeks.
//!
//! Every Places call in this app must run the parsed body through
//! [`check_response`]. It logs the HTTP status, Google's own `status` and Google's
//! own `error_message` at **WARN** (never the request URL — that carries the key),
//! and returns a structured [`PlacesError`] the caller can surface to the API
//! response, the enrichment log and the admin UI.
//!
//! A `ZERO_RESULTS` (or an empty result array) is NOT an error — it is an honest
//! "no such business", and it is logged at INFO so the two can never be confused.

use serde_json::Value;

/// The two Places `status` values that mean "the provider answered us": `OK`
/// (a reply, possibly empty) and `ZERO_RESULTS` (a reply that found nothing).
pub const PLACES_NON_ERROR_STATUSES: [&str; 2] = ["OK", "ZERO_RESULTS"];

/// Which Places sub-API was called — used as the log/response context so an error
/// names the endpoint without ever printing the URL (which contains the API key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacesCall {
    Autocomplete,
    FindPlace,
    TextSearch,
    Details,
}

impl PlacesCall {
    pub fn as_str(self) -> &'static str {
        match self {
            PlacesCall::Autocomplete => "place/autocomplete",
            PlacesCall::FindPlace => "place/findplacefromtext",
            PlacesCall::TextSearch => "place/textsearch",
            PlacesCall::Details => "place/details",
        }
    }
}

/// A refused or failed Places call, carrying everything an operator needs to fix
/// it and nothing that leaks the key.
#[derive(Debug, Clone)]
pub struct PlacesError {
    /// Which Places endpoint refused us (`place/findplacefromtext`, …).
    pub call: String,
    /// The HTTP status the transport returned (usually 200 even on refusal).
    pub http_status: u16,
    /// Google's own `status` field, or `"UNKNOWN"` when the body had none.
    pub provider_status: String,
    /// Google's own `error_message`, when it sent one.
    pub provider_error: Option<String>,
}

impl std::fmt::Display for PlacesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "google_places {} refused: HTTP {} provider_status={} error={}",
            self.call,
            self.http_status,
            self.provider_status,
            self.provider_error.as_deref().unwrap_or("-")
        )
    }
}

impl std::error::Error for PlacesError {}

// Let handlers use `?` on a Places call: the provider's own message travels up to the API response
// and the log instead of being swallowed. A rejected key or an exceeded quota must be visible —
// invisible provider errors are what made a working key look broken (card B78).
impl From<PlacesError> for crate::error::AppError {
    fn from(e: PlacesError) -> Self {
        crate::error::AppError::BadRequest(e.to_string())
    }
}

impl PlacesError {
    /// The distinguishable outcome tag for an API response — `provider_error` when
    /// Google named a status (key rejected, quota, bad request), `transport_error`
    /// when we never got a parseable Places answer at all.
    pub fn outcome(&self) -> &'static str {
        if self.provider_status.is_empty() || self.provider_status == "UNKNOWN" {
            "transport_error"
        } else {
            "provider_error"
        }
    }
}

/// Google's `status` from a Places body, upper-cased and trimmed. Missing ⇒ `"UNKNOWN"`.
pub fn provider_status(body: &Value) -> String {
    body.get("status")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

/// Google's `error_message` from a Places body, trimmed.
pub fn provider_error(body: &Value) -> Option<String> {
    body.get("error_message")
        .and_then(|m| m.as_str())
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
}

/// Inspect a Places response. `Ok(())` means Google answered (`OK`/`ZERO_RESULTS`;
/// an empty result set is returned as `Ok` and the caller reports a real miss).
/// `Err` means Google REFUSED or the transport failed — always logged at WARN with
/// the HTTP status, Google's `status` and Google's `error_message`, never the URL.
pub fn check_response(body: &Value, http_status: u16, call: PlacesCall) -> Result<(), PlacesError> {
    let status = provider_status(body);
    let err = provider_error(body);

    let transport_ok = (200..300).contains(&http_status);
    let answered = PLACES_NON_ERROR_STATUSES.contains(&status.as_str());

    if transport_ok && answered {
        if status == "ZERO_RESULTS" {
            tracing::info!(
                "[places] {} — HTTP {} status=ZERO_RESULTS (honest no-result, not an error)",
                call.as_str(),
                http_status
            );
        }
        return Ok(());
    }

    let e = PlacesError {
        call: call.as_str().to_string(),
        http_status,
        provider_status: status,
        provider_error: err,
    };
    tracing::warn!("[places] {}", e);
    Err(e)
}

/// Build a `PlacesError` for a response we could not even parse (non-JSON body,
/// connection failure): the caller passes the transport detail as the message.
pub fn transport_error(call: PlacesCall, http_status: u16, detail: &str) -> PlacesError {
    let e = PlacesError {
        call: call.as_str().to_string(),
        http_status,
        provider_status: "UNKNOWN".to_string(),
        provider_error: Some(detail.to_string()),
    };
    tracing::warn!("[places] {}", e);
    e
}

// ─────────────────────────────────────────────────────────────────────────────
// The two-step Places lookup — the fix for the malformed enrichment request (B80)
// ─────────────────────────────────────────────────────────────────────────────

/// The legacy Places endpoint root.
pub const LEGACY_PLACES_BASE: &str = "https://maps.googleapis.com/maps/api/place";

/// The field names **Find Place From Text** actually accepts.
///
/// Verified against the live API (2026-09-30): `Find Place` answers
///
/// ```text
/// status = INVALID_REQUEST
/// error_message = "Error while parsing 'fields' parameter: Unsupported field name 'formatted_phone_number'."
/// ```
///
/// for `formatted_phone_number`, `website` and `url`. Phone, website, opening hours and address
/// components are **Place Details** fields. Asking Find Place for them made EVERY enrichment
/// return `matched:false` (Google refused the request, so no candidate ever came back) while the
/// key and the provider were fine. Deleting the names would silently lose phone/website, so the
/// data is fetched with a second call instead — see [`find_place_then_details`].
pub const FIND_PLACE_FIELDS: &str =
    "place_id,name,formatted_address,geometry,types,photos,business_status";

/// The rich **Place Details** field set: everything enrichment wants from a resolved place.
/// All of these are valid Details field names (also verified live).
pub const PLACE_DETAILS_FIELDS: &str = "place_id,name,formatted_address,formatted_phone_number,website,geometry,rating,user_ratings_total,types,photos,opening_hours,business_status,address_components,url,vicinity";

/// One resolved place: the Find Place candidate plus the Place Details result, carrying the HTTP
/// status of each call so a caller can report exactly where an answer came from.
#[derive(Debug, Clone)]
pub struct PlacesLookup {
    pub candidate: Value,
    pub details: Value,
    pub find_http_status: u16,
    pub details_http_status: u16,
    /// Google's own envelope `status` from the Find Place call (`"OK"` when it answered).
    pub provider_status: String,
}

/// The outcome of a two-step lookup. `NoMatch` carries Google's own envelope status so a real
/// "no such listing" is reported with the answer Google actually gave, never a guess.
#[derive(Debug, Clone)]
pub enum PlacesOutcome {
    Found(Box<PlacesLookup>),
    NoMatch { provider_status: String },
}

impl PlacesLookup {
    /// The place id Find Place resolved — the join key between the two calls.
    pub fn place_id(&self) -> Option<&str> {
        self.candidate
            .get("place_id")
            .or_else(|| self.details.get("place_id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Candidate fields overlaid with the (non-null) Details fields: the richest answer
    /// available. Sibling fields such as `field_sources` say which call supplied each value.
    pub fn merged(&self) -> Value {
        let mut m = self.candidate.clone();
        if let (Some(mo), Some(d)) = (m.as_object_mut(), self.details.as_object()) {
            for (k, v) in d {
                if !v.is_null() {
                    mo.insert(k.clone(), v.clone());
                }
            }
        }
        m
    }

    /// Provenance for the admin UI: which Places call produced each returned field, e.g.
    /// `{"formatted_phone_number":"place/details"}`. Only fields actually returned appear.
    pub fn field_sources(&self) -> Value {
        let mut out = serde_json::Map::new();
        if let Some(c) = self.candidate.as_object() {
            for (k, v) in c {
                if !v.is_null() {
                    out.insert(
                        k.clone(),
                        Value::String(PlacesCall::FindPlace.as_str().to_string()),
                    );
                }
            }
        }
        if let Some(d) = self.details.as_object() {
            for (k, v) in d {
                if !v.is_null() {
                    out.insert(
                        k.clone(),
                        Value::String(PlacesCall::Details.as_str().to_string()),
                    );
                }
            }
        }
        Value::Object(out)
    }
}

/// Percent-encode a query value for a Places URL (query-string form: space ⇒ `+`).
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// GET a Places endpoint, returning `(http_status, parsed_body)`. A transport failure or a
/// non-JSON body is a [`PlacesError`], never an empty body (card B78).
async fn http_get_json(url: &str, call: PlacesCall) -> Result<(u16, Value), PlacesError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| transport_error(call, 0, &format!("http client: {}", e)))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| transport_error(call, 0, &format!("request failed: {}", e)))?;
    let http_status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| transport_error(call, http_status, &format!("body read failed: {}", e)))?;
    let body = serde_json::from_str::<Value>(&text).map_err(|e| {
        transport_error(
            call,
            http_status,
            &format!(
                "non-JSON body: {} | {}",
                e,
                text.chars().take(160).collect::<String>()
            ),
        )
    })?;
    Ok((http_status, body))
}

/// **The correct Google Places lookup for enrichment**: Find Place From Text resolves the
/// candidate using only the fields it supports, then Place Details (by `place_id`) supplies
/// phone / website / opening hours / rating / address components — the fields Find Place
/// REJECTS (card B80).
///
/// `Ok(NoMatch)` = Google answered and there is no such listing (an honest miss, reported with
/// Google's own status). `Err` = a refusal or transport failure, naming which of the two calls
/// refused. The two are never conflated.
pub async fn find_place_then_details(
    base_url: &str,
    api_key: &str,
    input: &str,
) -> Result<PlacesOutcome, PlacesError> {
    let base = base_url.trim_end_matches('/');
    let find_url = format!(
        "{}/findplacefromtext/json?input={}&inputtype=textquery&fields={}&key={}",
        base,
        enc(input.trim()),
        FIND_PLACE_FIELDS,
        api_key
    );
    let (find_status, find_body) = http_get_json(&find_url, PlacesCall::FindPlace).await?;
    check_response(&find_body, find_status, PlacesCall::FindPlace)?;

    let find_provider_status = provider_status(&find_body);
    let candidate = find_body
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned();
    let place_id = candidate
        .as_ref()
        .and_then(|c| c.get("place_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let Some(candidate) = candidate else {
        return Ok(PlacesOutcome::NoMatch {
            provider_status: find_provider_status,
        });
    };
    let Some(place_id) = place_id else {
        return Ok(PlacesOutcome::NoMatch {
            provider_status: find_provider_status,
        });
    };

    let details_url = format!(
        "{}/details/json?place_id={}&fields={}&key={}",
        base,
        enc(&place_id),
        PLACE_DETAILS_FIELDS,
        api_key
    );
    let (details_status, details_body) = http_get_json(&details_url, PlacesCall::Details).await?;
    check_response(&details_body, details_status, PlacesCall::Details)?;

    // An empty `result` is not an error — fall back to the candidate so the name/address the
    // Find Place call already returned still reaches the caller.
    let details = details_body
        .get("result")
        .cloned()
        .filter(|v| v.as_object().map(|o| !o.is_empty()).unwrap_or(false))
        .unwrap_or(Value::Null);

    Ok(PlacesOutcome::Found(Box::new(PlacesLookup {
        candidate,
        details,
        find_http_status: find_status,
        details_http_status: details_status,
        provider_status: find_provider_status,
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Defensible matching — a wrong phone number on a listing is worse than an empty one (B80)
// ─────────────────────────────────────────────────────────────────────────────

/// Significant name tokens: lower-cased alphanumeric runs, minus legal-suffix noise.
fn name_tokens(s: &str) -> std::collections::BTreeSet<String> {
    const STOP: [&str; 11] = [
        "the", "and", "of", "llc", "inc", "co", "ltd", "corp", "pa", "pllc", "llp",
    ];
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !STOP.contains(t))
        .map(String::from)
        .collect()
}

fn norm_name(s: &str) -> String {
    name_tokens(s).into_iter().collect::<Vec<_>>().join(" ")
}

/// The ZIP (last 5-digit run) and the locality component in front of the state/ZIP component,
/// e.g. `"2805 Clearwater Rd, St Cloud, MN 56301, USA"` → `(Some("56301"), Some("St Cloud"))`.
fn address_parts(addr: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = addr.split(',').map(str::trim).collect();
    let zip = parts
        .iter()
        .rev()
        .find_map(|p| {
            p.split(|c: char| !c.is_ascii_digit())
                .find(|d| d.len() == 5)
        })
        .map(String::from);
    let mut city = None;
    for (i, p) in parts.iter().enumerate() {
        let is_state_component = match p.split_whitespace().next() {
            Some(t) => t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic()),
            None => false,
        };
        if is_state_component && i > 0 {
            let c = parts[i - 1].trim();
            if c.chars().any(|ch| ch.is_alphabetic()) {
                city = Some(c.to_string());
            }
            break;
        }
    }
    (zip, city)
}

fn norm_city(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn name_agrees(dir_name: &str, cand_name: &str) -> bool {
    let a = name_tokens(dir_name);
    let b = name_tokens(cand_name);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (as_, bs) = (norm_name(dir_name), norm_name(cand_name));
    if as_ == bs || as_.contains(&bs) || bs.contains(&as_) {
        return true;
    }
    let shared: Vec<&String> = a.intersection(&b).collect();
    let min = a.len().min(b.len());
    shared.len() * 2 >= min && shared.iter().any(|t| t.len() >= 4)
}

fn locale_agrees(dir_addr: &str, cand_addr: &str, dir_state: &str) -> bool {
    let (dz, dc) = address_parts(dir_addr);
    let (cz, cc) = address_parts(cand_addr);
    if let (Some(a), Some(b)) = (&dz, &cz) {
        // A ZIP is the strongest address check: the same ZIP means the same locality.
        return a == b;
    }
    if let (Some(a), Some(b)) = (&dc, &cc) {
        let (a, b) = (norm_city(a), norm_city(b));
        return !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a));
    }
    let st = dir_state.trim().to_lowercase();
    st.len() == 2
        && cand_addr
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|t| t == st)
}

/// Does Google's candidate defensibly describe the SAME business as the directory row?
///
/// The names must agree (one normalized name contains the other, or the shorter token set is at
/// least half-shared including one 4+ character token) **and** the addresses must agree (same ZIP
/// when both carry one, else the same locality, else the same state). With no directory address
/// to corroborate, only a near-exact name is accepted. Refusing an ambiguous candidate is the
/// point: auto-writing a stranger's phone number onto a listing is worse than leaving it blank.
#[must_use]
pub fn listing_agrees(
    dir_name: &str,
    dir_address: &str,
    dir_state: &str,
    cand_name: &str,
    cand_address: &str,
) -> bool {
    if !name_agrees(dir_name, cand_name) {
        return false;
    }
    let da = dir_address.trim();
    let ca = cand_address.trim();
    if da.is_empty() {
        // Nothing to corroborate the location with — require an effectively exact name.
        return !ca.is_empty() && norm_name(dir_name) == norm_name(cand_name);
    }
    if ca.is_empty() {
        return false;
    }
    locale_agrees(da, ca, dir_state)
}
