//! Business Dashboard — content mention metrics for business owners.
//! BL13 extension: per-business counts from blog posts, business articles,
//! and trap door (programmatic) pages.

use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::portal::{BusinessProfile, BusinessSubscriptionInfo};
use crate::AppState;

// ── Response Types ──

#[derive(Debug, Serialize)]
pub struct MentionCounts {
    pub this_week: i64,
    pub this_month: i64,
    pub all_time: i64,
}

/// Rolled-up performance totals for a business's programmatic ("city") pages:
/// the SUM of impressions / clicks / conversions across every published page
/// that mentions the business. Backs the compact "views · clicks · leads"
/// totals on the Mentions & Visibility card (work-queue line 86/90).
#[derive(Debug, Serialize)]
pub struct PageAnalytics {
    pub impressions: i64,
    pub clicks: i64,
    pub conversions: i64,
}

#[derive(Debug, Serialize)]
pub struct BusinessMetrics {
    pub blog_post_mentions: MentionCounts,
    pub business_article_mentions: MentionCounts,
    pub trap_door_mentions: MentionCounts,
    pub trap_door_analytics: PageAnalytics,
    pub total_mentions: MentionCounts,
}

#[derive(Debug, Serialize)]
pub struct ClaimedBusinessDashboard {
    pub business: BusinessProfile,
    pub metrics: BusinessMetrics,
    pub subscription: Option<BusinessSubscriptionInfo>,
}

#[derive(Debug, Serialize)]
pub struct BusinessDashboardResponse {
    pub claimed_businesses: Vec<ClaimedBusinessDashboard>,
}

// ── Handler ──

/// GET /api/v1/portal/business/dashboard
///
/// Returns each claimed business for the authenticated user along with
/// content mention metrics (blog posts, business articles, trap door pages).
pub async fn business_dashboard(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // ── Find claimed businesses for this user ──
    // The public claim route records the owner's EMAIL only (it runs before the claimant has an
    // account), and `/auth/login` resolves every business-portal signup from the `users` table —
    // so this query matching `user_id` alone returned an empty list for exactly those owners: the
    // portal then rendered "No claimed business on this account", the business picker stayed
    // empty, and the review Approve/Reject buttons were unreachable even though
    // `/business/reviews/pending` (which resolves ownership by email) listed the reviews.
    // Measured 2026-11-05: a freshly registered owner who claimed a listing saw
    // `{"claimed_businesses":[]}` while the same token could approve a review over the API.
    // Same three ownership branches as `can_moderate`/`owned_by` in handlers/reviews.rs.
    let caller_email: Option<String> = crate::owner_identity::caller_email(&s.db, user_id).await?;

    let claims_rows = sqlx::query_as::<_, ClaimedBusinessRow>(
        r#"SELECT id, business_id, owner_email, owner_name, owner_phone, user_id, is_active, created_at
           FROM claimed_businesses
           WHERE user_id = $1
              OR (is_active AND visitor_account_id = $1)
              OR (is_active AND visitor_account_id IS NULL AND user_id IS NULL
                  AND $2 IS NOT NULL AND lower(owner_email) = lower($2))
           ORDER BY created_at DESC"#,
    )
    .bind(user_id)
    .bind(&caller_email)
    .fetch_all(&s.db)
    .await?;

    let mut dashboard_items = Vec::new();

    for claim in &claims_rows {
        // ── Fetch business details (with category name from join) ──
        let biz = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<serde_json::Value>)>(
            r#"SELECT b.id, b.name, dc.name as category, b.city, b.state, b.phone, b.website, b.images
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.id = $1"#
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        let business_profile = match biz {
            Some((id, name, category, city, state, phone, website, images)) => BusinessProfile {
                id,
                name,
                category,
                city,
                state,
                phone,
                website,
                images,
            },
            None => continue,
        };

        let business_id = business_profile.id;

        // ── Query mention counts ──
        let blog_all = count_blog_mentions(&s.db, business_id, MentionInterval::All).await?;
        let blog_week = count_blog_mentions(&s.db, business_id, MentionInterval::Days7).await?;
        let blog_month = count_blog_mentions(&s.db, business_id, MentionInterval::Days30).await?;

        let article_all = count_article_mentions(&s.db, business_id, MentionInterval::All).await?;
        let article_week =
            count_article_mentions(&s.db, business_id, MentionInterval::Days7).await?;
        let article_month =
            count_article_mentions(&s.db, business_id, MentionInterval::Days30).await?;

        let trapdoor_all =
            count_trapdoor_mentions(&s.db, business_id, MentionInterval::All).await?;
        let trapdoor_week =
            count_trapdoor_mentions(&s.db, business_id, MentionInterval::Days7).await?;
        let trapdoor_month =
            count_trapdoor_mentions(&s.db, business_id, MentionInterval::Days30).await?;

        let trap_door_analytics = sum_trapdoor_analytics(&s.db, business_id).await?;

        let metrics = BusinessMetrics {
            blog_post_mentions: MentionCounts {
                this_week: blog_week,
                this_month: blog_month,
                all_time: blog_all,
            },
            business_article_mentions: MentionCounts {
                this_week: article_week,
                this_month: article_month,
                all_time: article_all,
            },
            trap_door_mentions: MentionCounts {
                this_week: trapdoor_week,
                this_month: trapdoor_month,
                all_time: trapdoor_all,
            },
            trap_door_analytics,
            total_mentions: MentionCounts {
                this_week: blog_week + article_week + trapdoor_week,
                this_month: blog_month + article_month + trapdoor_month,
                all_time: blog_all + article_all + trapdoor_all,
            },
        };

        // ── Fetch subscription info ──
        let sub = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<String>, Option<String>, Option<String>, Option<rust_decimal::Decimal>, Option<chrono::NaiveDate>, Option<chrono::NaiveDate>, Option<bool>)>(
            r#"SELECT bs.id, bs.tier_id, pt.name, bs.status, bs.billing_cycle, bs.price_paid, bs.start_date, bs.end_date, bs.auto_renew
               FROM business_subscriptions bs
               LEFT JOIN plan_tiers pt ON pt.id = bs.tier_id
               WHERE bs.business_id = $1
               ORDER BY bs.created_at DESC
               LIMIT 1"#
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        let subscription = sub.map(
            |(
                id,
                tier_id,
                tier_name,
                status,
                billing_cycle,
                price_paid,
                start_date,
                end_date,
                auto_renew,
            )| {
                BusinessSubscriptionInfo {
                    id,
                    tier_id,
                    tier_name,
                    status,
                    billing_cycle,
                    price_paid,
                    start_date,
                    end_date,
                    auto_renew,
                }
            },
        );

        dashboard_items.push(ClaimedBusinessDashboard {
            business: business_profile,
            metrics,
            subscription,
        });
    }

    Ok(Json(json!(BusinessDashboardResponse {
        claimed_businesses: dashboard_items,
    })))
}

// ── Helper query functions ──

/// GET /api/v1/portal/business/:business_id/page-links.csv
///
/// Downloads a CSV of every published programmatic ("city") page that mentions this
/// business — one row per page with its public URL, title, service, city and per-page
/// stats. This is the raw data behind the back-office "Download report" button
/// (work-queue line 86/90): the compact card shows the count, the detail lives here.
/// Owner-scoped via `can_moderate` (owner / claimant / directory owner / super_admin).
pub async fn business_page_links_csv(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<Response> {
    let biz_name: Option<String> = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_optional(&s.db)
        .await?;
    let biz_name = match biz_name {
        Some(n) => n,
        None => return Err(AppError::NotFound("Business not found".to_string())),
    };

    if !crate::handlers::reviews::can_moderate(&s.db, &claims, Some(business_id)).await? {
        return Err(AppError::Forbidden(
            "You can only download the report for a business you own or have claimed.".to_string(),
        ));
    }

    let rows = sqlx::query_as::<_, PageLinkRow>(
        r#"SELECT d.slug AS city_slug,
                  COALESCE(n.root_domain, '') AS root_domain,
                  pp.slug AS page_slug,
                  COALESCE(pp.title, '') AS title,
                  COALESCE(ds.name, '') AS service,
                  pp.impressions,
                  pp.clicks,
                  pp.conversions
           FROM programmatic_pages pp
           JOIN directories d ON d.id = pp.directory_id
           LEFT JOIN networks n ON n.id = d.network_id
           LEFT JOIN directory_services ds ON ds.id = pp.service_id
           WHERE $1 = ANY(pp.mentioned_business_ids) AND pp.status = 'published'
           ORDER BY d.slug, pp.slug"#,
    )
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    let mut csv = String::from("Page URL,Title,Service,City,Impressions,Clicks,Leads\n");
    for r in &rows {
        let base = if r.root_domain.is_empty() {
            format!("https://{}.{}", r.city_slug, s.config.base_domain)
        } else {
            format!("https://{}/{}", r.root_domain, r.city_slug)
        };
        let url = format!("{}/{}", base, r.page_slug);
        csv.push_str(&csv_line(&[
            &url,
            &r.title,
            &r.service,
            &r.city_slug,
            &r.impressions.to_string(),
            &r.clicks.to_string(),
            &r.conversions.to_string(),
        ]));
    }

    let filename = format!(
        "attachment; filename=\"zaarhub-page-links-{}.csv\"",
        safe_filename(&biz_name)
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, filename)
        .body(Body::from(csv))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// GET /api/v1/portal/business/:business_id/report.pdf
///
/// A generated CLIENT REPORT (PDF) for the period: every published programmatic
/// ("city") page that mentions the business, its public URL and its per-page
/// performance, plus rolled-up totals. Work-queue line 86 item (3) — the
/// downloadable report behind the back-office "Download report (PDF)" button.
/// Owner-scoped via `can_moderate`; the PDF is rendered in-process with the
/// `printpdf` built-in fonts, so there is no external renderer and no shell-out.
pub async fn business_report_pdf(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<Response> {
    let biz_name: Option<String> = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_optional(&s.db)
        .await?;
    let biz_name = match biz_name {
        Some(n) => n,
        None => return Err(AppError::NotFound("Business not found".to_string())),
    };

    if !crate::handlers::reviews::can_moderate(&s.db, &claims, Some(business_id)).await? {
        return Err(AppError::Forbidden(
            "You can only download the report for a business you own or have claimed.".to_string(),
        ));
    }

    let rows = sqlx::query_as::<_, PageLinkRow>(
        r#"SELECT d.slug AS city_slug,
                  COALESCE(n.root_domain, '') AS root_domain,
                  pp.slug AS page_slug,
                  COALESCE(pp.title, '') AS title,
                  COALESCE(ds.name, '') AS service,
                  pp.impressions,
                  pp.clicks,
                  pp.conversions
           FROM programmatic_pages pp
           JOIN directories d ON d.id = pp.directory_id
           LEFT JOIN networks n ON n.id = d.network_id
           LEFT JOIN directory_services ds ON ds.id = pp.service_id
           WHERE $1 = ANY(pp.mentioned_business_ids) AND pp.status = 'published'
           ORDER BY d.slug, pp.slug"#,
    )
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    let totals = sum_trapdoor_analytics(&s.db, business_id).await?;
    let generated = chrono::Utc::now().format("%B %e, %Y").to_string();
    let bytes = build_report_pdf(&biz_name, &s.config.base_domain, &rows, &totals, &generated)
        .map_err(AppError::Internal)?;

    let filename = format!(
        "attachment; filename=\"zaarhub-client-report-{}.pdf\"",
        safe_filename(&biz_name)
    );

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/pdf")
        .header(header::CONTENT_DISPOSITION, filename)
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Build the report PDF entirely in-process. Each mentioned page is a compact
/// block — service · city, title, URL, then its views/clicks/leads — with a
/// header summary and a rule between blocks. Pages flow onto new sheets as
/// needed. Built-in Helvetica fonts, so no font file or renderer is required.
fn build_report_pdf(
    biz_name: &str,
    base_domain: &str,
    rows: &[PageLinkRow],
    totals: &PageAnalytics,
    generated: &str,
) -> Result<Vec<u8>, String> {
    use printpdf::{BuiltinFont, Color, Mm, PdfDocument, Rgb};

    let (doc, page_idx, layer_idx) =
        PdfDocument::new("ZaarHub Client Report", Mm(210.0), Mm(297.0), "Report");
    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| e.to_string())?;
    let font_bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| e.to_string())?;

    let black = Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None));
    let gray = Color::Rgb(Rgb::new(0.35, 0.35, 0.35, None));

    let mut layer = doc.get_page(page_idx).get_layer(layer_idx);
    let mut y: f32 = 272.0;

    // Header.
    layer.set_fill_color(black.clone());
    layer.use_text("Client Report", 16.0, Mm(15.0), Mm(y), &font_bold);
    y -= 8.0;
    layer.use_text(clip_str(biz_name, 80), 12.0, Mm(15.0), Mm(y), &font);
    y -= 6.0;
    layer.use_text(
        format!("Generated {} · ZaarHub", generated),
        9.0,
        Mm(15.0),
        Mm(y),
        &font,
    );
    y -= 4.0;
    layer.set_fill_color(gray.clone());
    layer.use_text(
        format!(
            "{} city page(s) · {} views · {} clicks · {} leads",
            rows.len(),
            totals.impressions,
            totals.clicks,
            totals.conversions
        ),
        10.0,
        Mm(15.0),
        Mm(y),
        &font_bold,
    );
    layer.set_fill_color(black.clone());
    y -= 5.0;

    draw_rule(&layer, y);
    y -= 9.0;

    // Per-page blocks.
    for r in rows {
        if y < 28.0 {
            let (p, l) = doc.add_page(Mm(210.0), Mm(297.0), "Report");
            layer = doc.get_page(p).get_layer(l);
            y = 272.0;
        }

        let heading = match (r.service.trim(), r.city_slug.trim()) {
            (s, c) if !s.is_empty() && !c.is_empty() => format!("{} · {}", s, c),
            (s, _) if !s.is_empty() => s.to_string(),
            (_, c) if !c.is_empty() => c.to_string(),
            _ => r.page_slug.clone(),
        };

        layer.set_fill_color(black.clone());
        layer.use_text(clip_str(&heading, 90), 9.5, Mm(15.0), Mm(y), &font_bold);
        y -= 4.2;

        layer.set_fill_color(gray.clone());
        layer.use_text(clip_str(&r.title, 95), 8.0, Mm(15.0), Mm(y), &font);
        y -= 3.8;

        layer.use_text(
            clip_str(&page_url(r, base_domain), 95),
            7.0,
            Mm(15.0),
            Mm(y),
            &font,
        );
        y -= 3.8;

        layer.set_fill_color(black.clone());
        layer.use_text(
            format!(
                "Views {} · Clicks {} · Leads {}",
                r.impressions, r.clicks, r.conversions
            ),
            8.0,
            Mm(15.0),
            Mm(y),
            &font,
        );
        y -= 5.0;

        draw_rule(&layer, y);
        y -= 7.0;
    }

    doc.save_to_bytes().map_err(|e| e.to_string())
}

/// Public URL for a programmatic page — network root domain when set, else the
/// city subdomain on the base domain.
fn page_url(r: &PageLinkRow, base_domain: &str) -> String {
    let base = if r.root_domain.is_empty() {
        format!("https://{}.{}", r.city_slug, base_domain)
    } else {
        format!("https://{}/{}", r.root_domain, r.city_slug)
    };
    format!("{}/{}", base, r.page_slug)
}

/// Draw a thin horizontal rule across the printable width at `y_mm`.
fn draw_rule(layer: &printpdf::PdfLayerReference, y_mm: f32) {
    layer.set_outline_color(printpdf::Color::Rgb(printpdf::Rgb::new(
        0.78, 0.78, 0.78, None,
    )));
    layer.set_outline_thickness(0.35);
    layer.add_line(printpdf::Line {
        points: vec![
            (
                printpdf::Point::new(printpdf::Mm(15.0), printpdf::Mm(y_mm)),
                false,
            ),
            (
                printpdf::Point::new(printpdf::Mm(195.0), printpdf::Mm(y_mm)),
                false,
            ),
        ],
        is_closed: false,
    });
}

/// Truncate to at most `max` chars, appending "..." — ASCII-only so it renders
/// under the built-in Helvetica (Windows-1252) encoding.
fn clip_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let clipped: String = s.chars().take(max).collect();
        format!("{}...", clipped)
    }
}

/// One published programmatic page that mentions a business, joined to its
/// directory (city) and network (brand host) so the CSV can print a real URL.
#[derive(Debug, sqlx::FromRow)]
struct PageLinkRow {
    city_slug: String,
    root_domain: String,
    page_slug: String,
    title: String,
    service: String,
    impressions: i32,
    clicks: i32,
    conversions: i32,
}

/// Quote a CSV field only when it contains a comma, quote, CR or LF (RFC 4180).
fn csv_line(fields: &[&str]) -> String {
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if f.contains(',') || f.contains('"') || f.contains('\n') || f.contains('\r') {
            out.push('"');
            out.push_str(&f.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(f);
        }
    }
    out.push('\n');
    out
}

/// Business name → filesystem-safe token for the Content-Disposition filename.
fn safe_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "business".to_string()
    } else {
        trimmed
    }
}

/// Time window for the dashboard mention counters. A closed compile-time set: each variant selects
/// a COMPLETE statement literal, so no SQL text is ever assembled at run time (class-14 paydown,
/// kanban t_faba8c76). The text is byte-identical to what the hand-built version sent per window.
#[derive(Clone, Copy)]
enum MentionInterval {
    All,
    Days7,
    Days30,
}

async fn count_blog_mentions(
    db: &sqlx::PgPool,
    business_id: Uuid,
    interval: MentionInterval,
) -> ApiResult<i64> {
    let sql: &str = match interval {
        MentionInterval::All => concat!(
            "SELECT COUNT(*) FROM blog_posts ",
            "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published'"
        ),
        MentionInterval::Days7 => concat!(
            "SELECT COUNT(*) FROM blog_posts ",
            "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published' ",
            "AND created_at >= NOW() - INTERVAL '7 days'"
        ),
        MentionInterval::Days30 => concat!(
            "SELECT COUNT(*) FROM blog_posts ",
            "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published' ",
            "AND created_at >= NOW() - INTERVAL '30 days'"
        ),
    };

    let (count,) = sqlx::query_as::<_, (i64,)>(sql)
        .bind(business_id)
        .fetch_one(db)
        .await?;

    Ok(count)
}

async fn count_article_mentions(
    db: &sqlx::PgPool,
    business_id: Uuid,
    interval: MentionInterval,
) -> ApiResult<i64> {
    let sql: &str = match interval {
        MentionInterval::All => concat!(
            "SELECT COUNT(*) FROM business_articles ",
            "WHERE business_id = $1 AND status = 'published'"
        ),
        MentionInterval::Days7 => concat!(
            "SELECT COUNT(*) FROM business_articles ",
            "WHERE business_id = $1 AND status = 'published' ",
            "AND created_at >= NOW() - INTERVAL '7 days'"
        ),
        MentionInterval::Days30 => concat!(
            "SELECT COUNT(*) FROM business_articles ",
            "WHERE business_id = $1 AND status = 'published' ",
            "AND created_at >= NOW() - INTERVAL '30 days'"
        ),
    };

    let (count,) = sqlx::query_as::<_, (i64,)>(sql)
        .bind(business_id)
        .fetch_one(db)
        .await?;

    Ok(count)
}

async fn count_trapdoor_mentions(
    db: &sqlx::PgPool,
    business_id: Uuid,
    interval: MentionInterval,
) -> ApiResult<i64> {
    let sql: &str = match interval {
        MentionInterval::All => concat!(
            "SELECT COUNT(*) FROM programmatic_pages ",
            "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published'"
        ),
        MentionInterval::Days7 => concat!(
            "SELECT COUNT(*) FROM programmatic_pages ",
            "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published' ",
            "AND created_at >= NOW() - INTERVAL '7 days'"
        ),
        MentionInterval::Days30 => concat!(
            "SELECT COUNT(*) FROM programmatic_pages ",
            "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published' ",
            "AND created_at >= NOW() - INTERVAL '30 days'"
        ),
    };

    let (count,) = sqlx::query_as::<_, (i64,)>(sql)
        .bind(business_id)
        .fetch_one(db)
        .await?;

    Ok(count)
}

/// SUM of impressions / clicks / conversions across every published programmatic
/// page that mentions the business. `SUM(integer)` yields bigint, so the row is
/// read as i64. A business mentioned by no page returns zeros (never an error).
async fn sum_trapdoor_analytics(db: &sqlx::PgPool, business_id: Uuid) -> ApiResult<PageAnalytics> {
    let (impressions, clicks, conversions) = sqlx::query_as::<_, (i64, i64, i64)>(concat!(
        "SELECT COALESCE(SUM(impressions), 0), COALESCE(SUM(clicks), 0), ",
        "COALESCE(SUM(conversions), 0) FROM programmatic_pages ",
        "WHERE $1 = ANY(mentioned_business_ids) AND status = 'published'"
    ))
    .bind(business_id)
    .fetch_one(db)
    .await?;

    Ok(PageAnalytics {
        impressions,
        clicks,
        conversions,
    })
}

// ── Local helper type (mirrors portal's ClaimedBusinessRow for local use) ──

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClaimedBusinessRow {
    pub business_id: Uuid,
}
