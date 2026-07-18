//! Public booking page — serves a standalone HTML booking form for a directory business.
//!
//! This endpoint renders a complete, mobile-responsive HTML page that:
//! 1. Shows business info (name, description, website)
//! 2. Displays available booking slots from CoreSwift
//! 3. Provides a booking form that POSTs to the create_booking API
//! 4. Uses inline CSS (no external dependencies)

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    http::StatusCode,
};
use serde_json::Value;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::coreswift::coreswift_url;

lazy_static::lazy_static! {
    static ref HTTP: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build reqwest client");
}

/// GET /api/v1/book/:slug/:business_id
///
/// Renders a public HTML booking page for a business in a directory.
/// Looks up directory + business info, fetches available slots from CoreSwift,
/// and returns a complete HTML page with an inline booking form.
pub async fn booking_page(
    State(s): State<AppState>,
    Path((slug, business_id_or_slug)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    // 1. Look up directory by slug
    let directory = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<String>, String)>(
        "SELECT id, coreswift_tenant_id, booking_calendar_slug, name FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let (dir_id, tenant_id_opt, calendar_slug_opt, dir_name) = directory;

    let tenant_id = tenant_id_opt
        .ok_or(AppError::BadRequest(format!(
            "Directory '{}' has no CoreSwift tenant configured", slug
        )))?;

    // 2. Look up business by UUID or slug
    let business = if let Ok(bid) = Uuid::parse_str(&business_id_or_slug) {
        sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>)>(
            "SELECT id, name, description, website FROM businesses WHERE id = $1 AND directory_id = $2"
        )
        .bind(bid)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>)>(
            "SELECT id, name, description, website FROM businesses WHERE slug = $1 AND directory_id = $2"
        )
        .bind(&business_id_or_slug)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    };

    let (business_id, business_name, description, website) = business
        .ok_or(AppError::NotFound("Business not found".to_string()))?;

    // 3. Fetch available slots from CoreSwift
    let base = coreswift_url();
    let slots_json = match HTTP
        .get(format!("{}/api/public/bookings/public/slots/available/{}", base, tenant_id))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                resp.json::<Value>().await.unwrap_or_else(|_| serde_json::json!({"error": "parse failed"}))
            } else {
                serde_json::json!({"error": format!("CoreSwift returned {}", status)})
            }
        }
        Err(e) => serde_json::json!({"error": format!("{}", e)}),
    };

    // 4. Extract slot info for display
    let slots_display = format_slots_for_display(&slots_json);
    let has_slots = !slots_display.contains("No slots");

    // 5. Build the available slots section HTML
    let slots_section = if has_slots {
        format!(
            r#"<div class="slots-section">
                <h3>Available Appointment Slots</h3>
                <div class="slots-info">
                    <p>This business has <strong>available booking slots</strong>. Select a date and time below to book.</p>
                    <pre class="slots-raw">{}</pre>
                </div>
            </div>"#,
            html_escape(&slots_display)
        )
    } else {
        r#"<div class="slots-section">
            <h3>Available Appointment Slots</h3>
            <div class="slots-info slots-unavailable">
                <p>No specific slot types are configured. You can still book — just fill out the form and the business will be notified.</p>
            </div>
        </div>"#.to_string()
    };

    // 6. Build the booking endpoint URL for the form action
    let booking_endpoint = format!("/api/v1/directories/{}/businesses/{}/book", slug, business_id);

    // 7. Render the complete HTML page
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Book {} - {}</title>
    <style>
        *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            line-height: 1.6;
            color: #1a1a2e;
            background: linear-gradient(135deg, #f5f7fa 0%, #c3cfe2 100%);
            min-height: 100vh;
        }}
        .container {{
            max-width: 800px;
            margin: 0 auto;
            padding: 2rem 1rem;
        }}
        .card {{
            background: #fff;
            border-radius: 16px;
            box-shadow: 0 10px 40px rgba(0,0,0,0.08);
            overflow: hidden;
        }}
        .card-header {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #fff;
            padding: 2rem;
        }}
        .card-header h1 {{
            font-size: 1.8rem;
            margin-bottom: 0.5rem;
        }}
        .card-header p {{
            opacity: 0.9;
            font-size: 0.95rem;
        }}
        .card-header .dir-name {{
            display: inline-block;
            background: rgba(255,255,255,0.2);
            padding: 0.2rem 0.8rem;
            border-radius: 20px;
            font-size: 0.8rem;
            margin-bottom: 0.8rem;
        }}
        .card-body {{
            padding: 2rem;
        }}
        .business-info {{
            margin-bottom: 2rem;
            padding-bottom: 1.5rem;
            border-bottom: 1px solid #eee;
        }}
        .business-info h2 {{
            font-size: 1.4rem;
            color: #333;
            margin-bottom: 0.5rem;
        }}
        .business-info .desc {{
            color: #666;
            margin-bottom: 0.5rem;
        }}
        .business-info .website {{
            display: inline-block;
            color: #667eea;
            text-decoration: none;
            font-size: 0.9rem;
        }}
        .business-info .website:hover {{
            text-decoration: underline;
        }}
        .slots-section {{
            margin-bottom: 2rem;
            padding-bottom: 1.5rem;
            border-bottom: 1px solid #eee;
        }}
        .slots-section h3 {{
            font-size: 1.1rem;
            color: #333;
            margin-bottom: 0.8rem;
        }}
        .slots-info {{
            background: #f0fdf4;
            border: 1px solid #bbf7d0;
            border-radius: 8px;
            padding: 1rem;
        }}
        .slots-unavailable {{
            background: #fefce8;
            border-color: #fde68a;
        }}
        .slots-raw {{
            margin-top: 0.5rem;
            font-size: 0.8rem;
            color: #555;
            white-space: pre-wrap;
            max-height: 200px;
            overflow-y: auto;
        }}
        .form-section h3 {{
            font-size: 1.1rem;
            color: #333;
            margin-bottom: 1.2rem;
        }}
        .form-group {{
            margin-bottom: 1.2rem;
        }}
        .form-group label {{
            display: block;
            font-weight: 600;
            font-size: 0.9rem;
            color: #333;
            margin-bottom: 0.4rem;
        }}
        .form-group input,
        .form-group textarea,
        .form-group select {{
            width: 100%;
            padding: 0.7rem 0.9rem;
            border: 1.5px solid #ddd;
            border-radius: 8px;
            font-size: 0.95rem;
            transition: border-color 0.2s;
            font-family: inherit;
        }}
        .form-group input:focus,
        .form-group textarea:focus,
        .form-group select:focus {{
            outline: none;
            border-color: #667eea;
            box-shadow: 0 0 0 3px rgba(102,126,234,0.15);
        }}
        .form-group textarea {{
            min-height: 80px;
            resize: vertical;
        }}
        .form-row {{
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 1rem;
        }}
        @media (max-width: 600px) {{
            .form-row {{
                grid-template-columns: 1fr;
            }}
        }}
        .btn {{
            display: inline-block;
            width: 100%;
            padding: 0.85rem 1.5rem;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #fff;
            border: none;
            border-radius: 8px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: transform 0.15s, box-shadow 0.15s;
        }}
        .btn:hover {{
            transform: translateY(-1px);
            box-shadow: 0 4px 15px rgba(102,126,234,0.4);
        }}
        .btn:active {{
            transform: translateY(0);
        }}
        .btn:disabled {{
            opacity: 0.6;
            cursor: not-allowed;
            transform: none;
        }}
        .alert {{
            border-radius: 8px;
            padding: 1rem;
            margin-bottom: 1.5rem;
            display: none;
        }}
        .alert-success {{
            background: #f0fdf4;
            border: 1px solid #bbf7d0;
            color: #166534;
            display: block;
        }}
        .alert-error {{
            background: #fef2f2;
            border: 1px solid #fecaca;
            color: #991b1b;
            display: block;
        }}
        .footer {{
            text-align: center;
            margin-top: 2rem;
            padding: 1rem;
            color: #888;
            font-size: 0.85rem;
        }}
        .spinner {{
            display: inline-block;
            width: 16px;
            height: 16px;
            border: 2px solid rgba(255,255,255,0.3);
            border-top-color: #fff;
            border-radius: 50%;
            animation: spin 0.6s linear infinite;
            vertical-align: middle;
            margin-right: 0.3rem;
        }}
        @keyframes spin {{
            to {{ transform: rotate(360deg); }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="card">
            <div class="card-header">
                <div class="dir-name">{}</div>
                <h1>Book an Appointment</h1>
                <p>Fill out the form below to schedule a time with <strong>{}</strong></p>
            </div>
            <div class="card-body">
                <!-- Business Info -->
                <div class="business-info">
                    <h2>{}</h2>
                    <div class="desc">{}</div>
                    {}
                </div>

                <!-- Available Slots -->
                {}

                <!-- Booking Form -->
                <div class="form-section">
                    <h3>Your Details</h3>
                    <div id="form-alert" class="alert" style="display:none;"></div>
                    <form id="booking-form" onsubmit="return submitBooking(event)">
                        <div class="form-row">
                            <div class="form-group">
                                <label for="contact_name">Full Name *</label>
                                <input type="text" id="contact_name" name="contact_name" required placeholder="Your full name">
                            </div>
                            <div class="form-group">
                                <label for="contact_email">Email Address *</label>
                                <input type="email" id="contact_email" name="contact_email" required placeholder="your@email.com">
                            </div>
                        </div>
                        <div class="form-row">
                            <div class="form-group">
                                <label for="contact_phone">Phone Number</label>
                                <input type="tel" id="contact_phone" name="contact_phone" placeholder="(555) 123-4567">
                            </div>
                            <div class="form-group">
                                <label for="preferred_date">Preferred Date *</label>
                                <input type="date" id="preferred_date" name="preferred_date" required min="{}">
                            </div>
                        </div>
                        <div class="form-group">
                            <label for="preferred_time">Preferred Time</label>
                            <input type="time" id="preferred_time" name="preferred_time">
                        </div>
                        <div class="form-group">
                            <label for="notes">Notes / Special Requests</label>
                            <textarea id="notes" name="notes" placeholder="Any specific requirements or information..."></textarea>
                        </div>
                        <button type="submit" class="btn" id="submit-btn">
                            <span id="btn-text">Request Appointment</span>
                            <span id="btn-spinner" class="spinner" style="display:none;"></span>
                        </button>
                    </form>
                </div>
            </div>
        </div>
        <div class="footer">
            Powered by <strong>DirectorySwift</strong>
        </div>
    </div>

    <script>
        async function submitBooking(event) {{
            event.preventDefault();
            const btn = document.getElementById('submit-btn');
            const btnText = document.getElementById('btn-text');
            const spinner = document.getElementById('btn-spinner');
            const alert = document.getElementById('form-alert');

            // Disable button
            btn.disabled = true;
            btnText.textContent = 'Submitting...';
            spinner.style.display = 'inline-block';
            alert.style.display = 'none';
            alert.className = 'alert';

            const form = document.getElementById('booking-form');
            const data = {{
                contact_name: form.contact_name.value.trim(),
                contact_email: form.contact_email.value.trim(),
                contact_phone: form.contact_phone.value.trim() || null,
                preferred_date: form.preferred_date.value,
                preferred_time: form.preferred_time.value || null,
                notes: form.notes.value.trim() || null,
            }};

            try {{
                const resp = await fetch('{}', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify(data),
                }});
                const result = await resp.json();
                if (resp.ok && result.success) {{
                    alert.className = 'alert alert-success';
                    alert.textContent = 'Booking confirmed! We'll be in touch soon.';
                    alert.style.display = 'block';
                    form.reset();
                    // Scroll to top to show success
                    window.scrollTo({{ top: 0, behavior: 'smooth' }});
                }} else {{
                    alert.className = 'alert alert-error';
                    alert.textContent = result.message || result.error || 'Booking failed. Please try again.';
                    alert.style.display = 'block';
                }}
            }} catch (err) {{
                alert.className = 'alert alert-error';
                alert.textContent = 'Network error. Please check your connection and try again.';
                alert.style.display = 'block';
            }} finally {{
                btn.disabled = false;
                btnText.textContent = 'Request Appointment';
                spinner.style.display = 'none';
            }}
            return false;
        }}

        // Set min date to today
        document.addEventListener('DOMContentLoaded', function() {{
            const dateInput = document.getElementById('preferred_date');
            if (dateInput) {{
                const today = new Date();
                const yyyy = today.getFullYear();
                const mm = String(today.getMonth() + 1).padStart(2, '0');
                const dd = String(today.getDate()).padStart(2, '0');
                dateInput.setAttribute('min', yyyy + '-' + mm + '-' + dd);
            }}
        }});
    </script>
</body>
</html>"#,
        html_escape(&business_name),
        html_escape(&dir_name),
        html_escape(&dir_name),
        html_escape(&business_name),
        html_escape(&business_name),
        html_escape(&description.clone().unwrap_or_default()),
        website_link(website.as_deref()),
        slots_section,
        today_date(),
        booking_endpoint,
    );

    Ok((StatusCode::OK, [("Content-Type", "text/html; charset=utf-8")], html))
}

/// Format the available slots JSON into a human-readable display string
fn format_slots_for_display(slots: &Value) -> String {
    if let Some(slots_array) = slots.as_array() {
        if slots_array.is_empty() {
            return "No slots available at this time.".to_string();
        }
        let mut lines = Vec::new();
        for slot in slots_array {
            let name = slot.get("name")
                .or_else(|| slot.get("slot_name"))
                .or_else(|| slot.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Appointment");
            let duration = slot.get("default_duration_days")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let total = slot.get("total_slots")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let note = if duration > 0 {
                format!(" ({})", pluralize("day", duration as usize))
            } else {
                String::new()
            };
            let capacity = if total == -1 {
                "Unlimited".to_string()
            } else {
                format!("{} {}", total, pluralize("slot", total as usize))
            };
            lines.push(format!("  - {}: {}{}", name, capacity, note));
        }
        lines.join("\n")
    } else if let Some(obj) = slots.as_object() {
        // Try to extract from various response formats
        if let Some(slots_array) = obj.get("slots").and_then(|v| v.as_array()) {
            return format_slots_for_display(&Value::Array(slots_array.clone()));
        }
        if let Some(slots_array) = obj.get("data").and_then(|v| v.as_array()) {
            return format_slots_for_display(&Value::Array(slots_array.clone()));
        }
        // Show raw JSON for debugging
        serde_json::to_string_pretty(obj).unwrap_or_else(|_| "Unable to parse slot data".to_string())
    } else {
        "No slot information available.".to_string()
    }
}

/// Pluralize a word
fn pluralize(word: &str, count: usize) -> String {
    if count == 1 { word.to_string() } else { format!("{}s", word) }
}

/// Simple HTML escaping
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Build a clickable website link or empty string
fn website_link(website: Option<&str>) -> String {
    match website {
        Some(url) if !url.trim().is_empty() => {
            let display = url.trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            format!(
                r#"<a class="website" href="{}" target="_blank" rel="noopener noreferrer">{}</a>"#,
                html_escape(url.trim()),
                html_escape(display)
            )
        }
        _ => String::new(),
    }
}

/// Get today's date in YYYY-MM-DD format
fn today_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Days since epoch
    let days = secs / 86400;
    // Calculate year/month/day (simple approach)
    let (year, month, day) = days_to_date(days as i64);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Convert days since epoch to (year, month, day)
fn days_to_date(days: i64) -> (i64, u32, u32) {
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }
    // Days in months for the given year
    let m_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in m_days.iter().enumerate() {
        if d < md {
            m = i;
            break;
        }
        d -= md;
    }
    (y, (m + 1) as u32, (d + 1) as u32)
}

/// Check if a year is a leap year
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
