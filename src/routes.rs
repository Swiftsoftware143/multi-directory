use axum::{
    routing::{get, post, put, delete},
    Router,
    middleware,
};
use std::sync::Arc;
use crate::AppState;
use crate::handlers::*;

pub fn create_router(s: AppState) -> Router {
    // ??? Public API routes (no auth needed)
    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route("/auth/login", post(auth_handler::login))
        .route("/auth/register", post(auth_handler::register))
        .route("/auth/forgot-password", post(auth_handler::forgot_password))
        .route("/auth/reset-password", post(auth_handler::reset_password))
        .route("/directories", get(directories::list_directories).post(directories::create_directory))
        .route("/directories/:slug", get(directories::get_directory).put(directories::update_directory).delete(directories::delete_directory))
        .route("/directories/:slug/render", get(directories::render_directory))
        .route("/directories/:slug/categories", get(directories::list_categories).post(directories::create_category))
        .route("/directories/:slug/categories/:category_id", put(directories::update_category).delete(directories::delete_category))
        .route("/directories/:slug/businesses", get(businesses::list_businesses).post(businesses::create_business))
        .route("/directories/:slug/businesses/suggestions", get(businesses::search_suggestions))
        .route("/directories/:slug/businesses/:business_id", get(businesses::get_business).put(businesses::update_business).delete(businesses::delete_business))
        .route("/reviews", get(reviews::list_reviews).post(reviews::create_review))
        .route("/reviews/:id", get(reviews::get_review).put(reviews::update_review).delete(reviews::delete_review))
        .route("/reviews/:id/approve", post(reviews::approve_review))
        .route("/reviews/:id/reject", post(reviews::reject_review))
        .route("/reviews/stats/:business_id", get(reviews::get_review_stats))
        .route("/directories/:slug/businesses/:business_id/reviews", get(reviews::list_business_reviews).post(reviews::create_review))
        .route("/directories/:slug/branding", get(branding::get_branding))
        .route("/templates", get(directories::list_templates))
        // ??? Blog routes (Phase 3)
        .route("/blog-posts", get(blog::list_blog_posts).post(blog::create_blog_post))
        .route("/blog-posts/:id", get(blog::get_blog_post).put(blog::update_blog_post).delete(blog::delete_blog_post))
        .route("/directories/:slug/blog-posts", get(blog::list_directory_blog_posts))
        // ??? Blog module aliases (Phase 3 Task 5)
        .route("/blog", get(blog::list_blog_posts).post(blog::create_blog_post))
        .route("/blog/:id", get(blog::get_blog_post).put(blog::update_blog_post).delete(blog::delete_blog_post))
        .route("/directories/:slug/blog", get(blog::list_directory_blog_posts))
        .route("/crm/contacts", get(crm::list_contacts).post(crm::create_contact))
        .route("/crm/contacts/:id", get(crm::get_contact).put(crm::update_contact).delete(crm::delete_contact))
        .route("/crm/contacts/search", get(crm::search_contacts))
        .route("/crm/pipelines", get(crm::list_pipelines).post(crm::create_pipeline))
        .route("/crm/pipelines/:id", get(crm::get_pipeline).put(crm::update_pipeline).delete(crm::delete_pipeline))
        .route("/crm/deals", get(crm::list_deals).post(crm::create_deal))
        .route("/crm/deals/:id", get(crm::get_deal).put(crm::update_deal).delete(crm::delete_deal))
        .route("/directories/:slug/crm/stats", get(crm::directory_crm_stats))
        .route("/legal-pages", get(legal::list_legal_pages).post(legal::create_legal_page))
        .route("/legal-pages/:id", get(legal::get_legal_page).put(legal::update_legal_page).delete(legal::delete_legal_page))
        .route("/deals", get(deals::list_deals).post(deals::create_deal))
        .route("/deals/featured", get(deals::list_featured_deals))
        .route("/deals/:id", get(deals::get_deal).put(deals::update_deal).delete(deals::delete_deal))
        .route("/deals/:id/claim", post(deals::claim_deal))
        .route("/directories/:slug/deals", get(deals::list_directory_deals))
        .route("/directories/:slug/businesses/:business_id/deals", get(deals::list_business_deals))
        .route("/submissions", get(submissions::list_submissions).post(submissions::create_submission))
        .route("/submissions/:id", get(submissions::get_submission).put(submissions::update_submission).delete(submissions::delete_submission))
        .route("/submissions/:id/approve", post(submissions::approve_submission))
        .route("/submissions/:id/reject", post(submissions::reject_submission))
        // ??? SEO routes
        .route("/seo/:page_type/:page_id", get(seo::get_seo_meta).put(seo::update_seo_meta))
        .route("/seo/sitemap-config", get(seo::list_all_sitemap_configs))
        .route("/seo/sitemap-config/:directory_id", get(seo::get_sitemap_config).put(seo::update_sitemap_config))
        .route("/seo/regenerate-sitemap", post(seo::regenerate_sitemap))
        .route("/sitemap.xml", get(seo::generate_sitemap))
        .route("/robots.txt", get(seo::get_robots_txt))
        .route("/search/filters/:directory_id", get(search::get_filters))
        .route("/search/config", get(search::list_search_configs).post(search::create_search_config))
        .route("/search/config/:directory_id", get(search::get_search_config).put(search::update_search_config))
        .route("/search", get(search::search_businesses))
        .route("/categories", get(categories::list_all_categories))
        .route("/listings", get(businesses::list_all_businesses))
        // ??? Analytics routes (Phase 3 Task 2)
        .route("/analytics/track", post(analytics::track_event))
        .route("/analytics", get(analytics::list_events))
        .route("/analytics/by-directory/:directory_id", get(analytics::by_directory))
        .route("/analytics/summary", get(analytics::get_summary))
        .route("/analytics/events", get(analytics::list_events))
        .route("/analytics/events/old", delete(analytics::purge_old_events))
        // ??? Email routes
        .route("/email/templates", get(email::list_templates).post(email::create_template))
        .route("/email/templates/:id", get(email::get_template).put(email::update_template).delete(email::delete_template))
        .route("/email/campaigns", get(email::list_campaigns).post(email::create_campaign))
        .route("/email/campaigns/:id", get(email::get_campaign).put(email::update_campaign).delete(email::delete_campaign))
        .route("/email/campaigns/:id/send", post(email::send_campaign))
        // ??? Public / landing page routes
        .route("/public/homepage", get(public::homepage_data))
        .route("/public/:slug", get(public::directory_data))
        .route("/public/:slug/:business_id", get(public::business_data))
        .route("/landing-pages", get(public_pages::list_landing_pages).post(public_pages::create_landing_page))
        .route("/landing-pages/:id", get(public_pages::get_landing_page).put(public_pages::update_landing_page).delete(public_pages::delete_landing_page))
        .route("/landing-pages/:slug/publish", post(public_pages::toggle_publish))
        .route("/public-themes", get(public_pages::list_public_themes).post(public_pages::create_public_theme))
        .route("/public-themes/:id", get(public_pages::get_public_theme).put(public_pages::update_public_theme).delete(public_pages::delete_public_theme))
        // ??? Public Pages module (Phase 3 Task 4) - aliases at /public-pages
        .route("/public-pages", get(public_pages::list_landing_pages).post(public_pages::create_landing_page))
        .route("/public-pages/:id", get(public_pages::get_landing_page).put(public_pages::update_landing_page).delete(public_pages::delete_landing_page))
        .route("/directories/:slug/public-pages", get(public::list_directory_public_pages))
        .route("/public-pages/featured", get(public_pages::list_landing_pages))
        .route("/public-pages/:slug/publish", post(public_pages::toggle_publish))
        // ??? Import/Export routes
        .route("/import", post(import_export::import_data))
        .route("/import/logs", get(import_export::list_import_logs))
        .route("/import/logs/:id", get(import_export::get_import_log))
        .route("/export/businesses/:directory_id", get(import_export::export_businesses))
        .route("/export/reviews/:directory_id", get(import_export::export_reviews))
        .route("/export/contacts/:directory_id", get(import_export::export_contacts))
        .route("/export/templates", get(import_export::list_export_templates).post(import_export::create_export_template))
        .route("/export/templates/:id", get(import_export::get_export_template).put(import_export::update_export_template).delete(import_export::delete_export_template))
        .route("/export/templates/:id/run", post(import_export::run_export_template))
        // ??? Monetization routes (Phase 3 Task 3) - aliases at /monetization
        .route("/monetization", get(monetization::monetization_dashboard))
        .route("/monetization/tiers", get(monetization::list_tiers).post(monetization::create_tier))
        .route("/monetization/tiers/:id", get(monetization::get_tier).put(monetization::update_tier).delete(monetization::delete_tier))
        .route("/monetization/subscriptions", get(monetization::list_subscriptions).post(monetization::create_subscription))
        .route("/monetization/subscriptions/:id", get(monetization::get_subscription).put(monetization::update_subscription).delete(monetization::delete_subscription))
        .route("/monetization/ad-zones", get(monetization::list_ad_zones).post(monetization::create_ad_zone))
        .route("/monetization/ad-zones/:id", get(monetization::get_ad_zone).put(monetization::update_ad_zone).delete(monetization::delete_ad_zone))
        // ??? Original monetization routes (keep backward compat)
        .route("/tiers", get(monetization::list_tiers).post(monetization::create_tier))
        .route("/tiers/:id", get(monetization::get_tier).put(monetization::update_tier).delete(monetization::delete_tier))
        .route("/subscriptions", get(monetization::list_subscriptions).post(monetization::create_subscription))
        .route("/subscriptions/:id", get(monetization::get_subscription).put(monetization::update_subscription).delete(monetization::delete_subscription))
        .route("/businesses/:id/subscription", get(monetization::business_subscription))
        .route("/ad-zones", get(monetization::list_ad_zones).post(monetization::create_ad_zone))
        .route("/ad-zones/:id", get(monetization::get_ad_zone).put(monetization::update_ad_zone).delete(monetization::delete_ad_zone))
        .route("/directories/:slug/ad-zones", get(monetization::directory_ad_zones))
        // ??? Directory tier routes (Phase 3 Task 3)
        .route("/monetization/directory-tiers", get(monetization::list_directory_tiers).post(monetization::create_directory_tier))
        .route("/monetization/directory-tiers/:id", get(monetization::get_directory_tier).put(monetization::update_directory_tier).delete(monetization::delete_directory_tier))
        .route("/monetization/directories/:slug/tier", get(monetization::directory_tier_by_slug))
        // ??? Sponsored listing routes (Phase 3 Task 3)
        .route("/monetization/sponsored-listings", get(monetization::list_sponsored_listings).post(monetization::create_sponsored_listing))
        .route("/monetization/sponsored-listings/:id", get(monetization::get_sponsored_listing).put(monetization::update_sponsored_listing).delete(monetization::delete_sponsored_listing))
        .route("/monetization/directories/:slug/sponsored-listings", get(monetization::directory_sponsored_listings))
        // ??? Call Tracking routes
        .route("/call-logs", get(call_tracking::list_call_logs).post(call_tracking::create_call_log))
        .route("/call-logs/:id", get(call_tracking::get_call_log))
        .route("/call-logs/:id/lead", put(call_tracking::update_call_lead))
        .route("/call-logs/stats", get(call_tracking::call_log_stats))
        .route("/directories/:slug/call-logs", get(call_tracking::directory_call_logs))
        .route("/businesses/:id/call-logs", get(call_tracking::business_call_logs))
        .route("/phone-numbers", get(call_tracking::list_phone_numbers).post(call_tracking::create_phone_number))
        .route("/phone-numbers/:id", get(call_tracking::get_phone_number).put(call_tracking::update_phone_number).delete(call_tracking::delete_phone_number))
        .route("/phone-numbers/:id/provision", post(call_tracking::provision_phone_number))
        // ??? Phase 4: Data Company — Google Places, verifications, enrichment, bulk export
        .route("/places/autocomplete", get(data_company::places_autocomplete))
        .route("/places/details", get(data_company::place_details))
        .route("/yelp/search", get(data_company::yelp_search))
        .route("/yelp/details", get(data_company::yelp_details))
        .route("/verifications", get(data_company::list_verifications).post(data_company::create_verification))
        .route("/verifications/:id", get(data_company::get_verification).put(data_company::update_verification))
        .route("/businesses/:id/verifications", get(data_company::business_verifications))
        .route("/enrich/business", post(data_company::enrich_business))
        .route("/enrich/logs", get(data_company::list_enrichment_logs))
        .route("/export/bulk", get(data_company::bulk_export))
        // ??? Phase 4: Automation — directory events, n8n bridge
        .route("/events", get(automation::list_events).post(automation::create_event))
        .route("/events/unprocessed", get(automation::unprocessed_events))
        .route("/events/:id/process", post(automation::mark_event_processed))
        .route("/n8n/webhook", post(automation::n8n_webhook_receiver))
        .route("/n8n/health", get(automation::n8n_health));

    // ??? Protected API routes (with auth middleware)
    let protected_routes = Router::new()
        .route("/auth/me", get(auth_handler::me))
        .route("/auth/password", put(auth_handler::change_password))
        .route("/dashboard/stats", get(admin::dashboard_stats))
        .route("/domains", get(domains::list_domains).post(domains::register_domain))
        .route("/domains/:domain_id", delete(domains::remove_domain))
        .route("/domains/:domain_id/verify", post(domains::verify_domain))
        .route("/branding/:directory_id", put(branding::update_branding))
        .route("/branding/:directory_id/extract", post(branding::extract_colors))
        .route("/portfolio/sync", post(admin::portfolio_sync))
        .route("/plans/:plan_id/domains", get(domains::check_plan_domains))
        // ??? Phase 4: API key management
        .route("/api-keys", get(api_complete::list_api_keys).post(api_complete::create_api_key))
        .route("/api-keys/:id", get(api_complete::get_api_key).put(api_complete::update_api_key).delete(api_complete::delete_api_key))
        .route("/api-keys/:id/usage", get(api_complete::get_api_key_usage))
        .route("/api-keys/verify", post(api_complete::verify_api_key))
        // ??? Phase 4: Webhook management
        .route("/webhooks", get(api_complete::list_webhooks).post(api_complete::create_webhook))
        .route("/webhooks/:id", get(api_complete::get_webhook).put(api_complete::update_webhook).delete(api_complete::delete_webhook))
        .route("/webhooks/:id/deliveries", get(api_complete::list_webhook_deliveries))
        .layer(middleware::from_fn_with_state(
            s.clone(),
            crate::auth::middleware::auth_middleware,
        ));

    // ??? Serve SPA frontend at root
    let frontend_dir = std::path::Path::new("/opt/swift/multidirectory-rust/frontend");
    let frontend_path = if frontend_dir.exists() {
        frontend_dir.to_string_lossy().to_string()
    } else {
        "./frontend".to_string()
    };

    // ??? Load SPA index.html into memory for fast fallback
    let index_path = std::path::Path::new(&frontend_path).join("index.html");
    let index_html = std::fs::read_to_string(&index_path).unwrap_or_else(|_| {
        "<!DOCTYPE html><html><head><title>Multi-Directory</title></head><body><h1>Multi-Directory</h1><p>App starting...</p></body></html>".to_string()
    });

    // Load login.html or fall back to index.html
    let login_path = std::path::Path::new(&frontend_path).join("login.html");
    let login_html = std::fs::read_to_string(&login_path).unwrap_or_else(|_| {
        index_html.clone()
    });

    let index_content: Arc<str> = Arc::from(index_html);
    let login_content: Arc<str> = Arc::from(login_html);

    // ??? Clone index_content for the second closure
    let index_content2 = index_content.clone();

    // ??? Combine: /api/v1/* API routes + static file server at /* + SPA fallback
    let app = Router::new()
        .nest("/api/v1", public_routes)
        .nest("/api/v1/admin", protected_routes)
        .fallback_service(
            tower::service_fn(move |req: axum::http::Request<axum::body::Body>| {
                let frontend = frontend_path.clone();
                let index_clone = index_content.clone();
                let login_clone = login_content.clone();
                let index_clone2 = index_content2.clone();
                async move {
                    let path = req.uri().path();

                    // ??? Serve clean login page for admin/login and login routes
                    if path == "/admin/login" || path == "/login" || path == "/admin/" || path == "/admin" {
                        return Ok::<_, std::convert::Infallible>(
                            axum::response::Response::builder()
                                .status(axum::http::StatusCode::OK)
                                .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                .body(axum::body::Body::from(login_clone.as_ref().to_string()))
                                .unwrap()
                        );
                    }

                    let clean_path = path.trim_start_matches('/');
                    let file_path = if clean_path.is_empty() {
                        std::path::Path::new(&frontend).join("index.html")
                    } else {
                        std::path::Path::new(&frontend).join(clean_path)
                    };

                    if file_path.exists() && file_path.is_file() {
                        match tokio::fs::read(&file_path).await {
                            Ok(content) => {
                                let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                                let mime = match ext {
                                    "html" => "text/html; charset=utf-8",
                                    "css" => "text/css; charset=utf-8",
                                    "js" => "application/javascript; charset=utf-8",
                                    "json" => "application/json",
                                    "png" => "image/png",
                                    "jpg" | "jpeg" => "image/jpeg",
                                    "svg" => "image/svg+xml",
                                    "ico" => "image/x-icon",
                                    "woff2" => "font/woff2",
                                    _ => "application/octet-stream",
                                };
                                return Ok::<_, std::convert::Infallible>(
                                    axum::response::Response::builder()
                                        .status(axum::http::StatusCode::OK)
                                        .header(axum::http::header::CONTENT_TYPE, mime)
                                        .body(axum::body::Body::from(content))
                                        .unwrap()
                                );
                            }
                            Err(_) => {}
                        }
                    }

                    // SPA fallback: serve full index.html for all unmatched routes
                    Ok(axum::response::Response::builder()
                        .status(axum::http::StatusCode::OK)
                        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                        .body(axum::body::Body::from(index_clone2.as_ref().to_string()))
                        .unwrap())
                }
            })
        )
        .with_state(s);

    app
}

/// GET /api/v1/health
async fn health_check() -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({
        "status": "ok",
        "service": "multidirectory-api",
        "version": env!("CARGO_PKG_VERSION")
    })))
}
