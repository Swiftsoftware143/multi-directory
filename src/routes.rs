use axum::{
    routing::{get, post, put, delete},
    Router,
    middleware::{self, Next},
    response::Response,
    extract::{Request, State},
};
use std::sync::Arc;
use crate::error::AppError;
use tracing::warn;
use crate::AppState;
use crate::handlers::*;

pub fn create_router(s: AppState) -> Router {
    // ??? Public API routes (no auth needed)
    let all_routes = Router::new()
        
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
        .route("/directories/:slug/email-settings", get(newsletter::get_email_settings).put(newsletter::upsert_email_settings).delete(newsletter::delete_email_settings))
        .route("/directories/:slug/subscribers", get(newsletter::list_subscribers).post(newsletter::add_subscriber))
        .route("/directories/:slug/subscribers/import", post(newsletter::import_subscribers))
        .route("/directories/:slug/subscribers/:id/unsubscribe", post(newsletter::unsubscribe_subscriber))
        .route("/templates", get(directories::list_templates))
        // ??? Blog routes (Phase 3)
        .route("/blog-posts", get(blog::list_blog_posts).post(blog::create_blog_post))
        .route("/blog-posts/:id", get(blog::get_blog_post).put(blog::update_blog_post).delete(blog::delete_blog_post))
        .route("/directories/:slug/blog-posts", get(blog::list_directory_blog_posts))
        // ??? Blog module aliases (Phase 3 Task 5)
        .route("/blog", get(blog::list_blog_posts).post(blog::create_blog_post))
        .route("/blog/:id", get(blog::get_blog_post).put(blog::update_blog_post).delete(blog::delete_blog_post))
        .route("/directories/:slug/blog", get(blog::list_directory_blog_posts))
        // Blog automation routes (Phase 5)
        .route("/blog-templates", get(blog::list_templates).post(blog::create_template))
        .route("/blog-templates/:id", get(blog::get_template).put(blog::update_template).delete(blog::delete_template))
        .route("/blog-posts/ext", get(blog::list_blog_posts_ext).post(blog::create_blog_post_ext))
        .route("/blog-posts/:id/ext", put(blog::update_blog_post_ext))
        .route("/blog-posts/:id/publish", post(blog::publish_blog_post_handler))
        .route("/blog-posts/scheduled", get(blog::list_scheduled_posts))
        .route("/blog/distribute", post(blog::distribute_blog_post))
        .route("/blog/process-scheduled", post(blog::process_scheduled_posts_handler))
        .route("/directories/:slug/blog-posts/ext", get(blog::list_directory_blog_posts_ext))
        .route("/newsletters", get(newsletter::list_newsletters).post(newsletter::create_newsletter))
        .route("/newsletters/:id", get(newsletter::get_newsletter).put(newsletter::update_newsletter).delete(newsletter::delete_newsletter))
        .route("/newsletters/:id/generate", post(newsletter::generate_newsletter_content))
        .route("/newsletters/:id/send", post(newsletter::send_newsletter))
        // Blog Generator (template-based AI content)
        .route("/blog-generate", post(blog_generator::generate_blog_posts))
        .route("/blog-posts/:id/regenerate", post(blog_generator::regenerate_blog_post))
        .route("/blog-templates/:id/directories", get(blog_generator::get_template_directories).post(blog_generator::set_template_directories))


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
        .route("/deals/:id/redeem", post(deals::redeem_deal))
        .route("/deals/:id/redemptions", get(deals::list_deal_redemptions))
        .route("/deals/redemptions/:rid/use", post(deals::use_redemption))
        .route("/deals/redemptions/expire", post(deals::expire_redemptions))
        .route("/deals/redemptions/code/:code", get(deals::lookup_redemption))
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
        .route("/search/suppliers", get(search::search_suppliers))
        .route("/categories", get(categories::list_all_categories))
        // Community posts (BL27)
        .route("/community/posts", get(blog::list_community_posts).post(blog::create_community_post))
        .route("/community/posts/:id", get(blog::get_blog_post).put(blog::update_community_post).delete(blog::delete_blog_post))
        // B2B Marketplace (Phase 4 — BL23)
        .route("/b2b/register", post(b2b::b2b_register))
        .route("/b2b/products", get(b2b::search_products).post(b2b::create_product))
        .route("/b2b/products/:id", get(b2b::get_product).put(b2b::update_product).delete(b2b::delete_product))
        .route("/b2b/suppliers", get(b2b::list_suppliers))
        // Supplier Portal (back office)
        .route("/supplier/profile", get(supplier::get_supplier_profile).put(supplier::update_supplier_profile))
        .route("/supplier/delivery", put(supplier::update_delivery_settings))
        .route("/supplier/featured-product", put(supplier::set_featured_product))
        // Scraper Engine — unified data import (BL15-21, BL24)
        .route("/scraper/providers", get(scraper::list_scraper_providers))
        .route("/scraper/import", post(scraper::data_import))
        .route("/scraper/google-places", post(scraper::scrape_google_places))
        .route("/listings", get(businesses::list_all_businesses))
        // ??? Analytics routes (Phase 3 Task 2)
        .route("/analytics/track", post(analytics::track_event))
        .route("/analytics", get(analytics::list_events))
        .route("/analytics/by-directory/:directory_id", get(analytics::by_directory))
        .route("/analytics/summary", get(analytics::get_summary))
        .route("/analytics/events", get(analytics::list_events))
        .route("/analytics/events/old", delete(analytics::purge_old_events))
        .route("/analytics/demand-curve", get(demand_curve::get_demand_curve))
        // ??? Email routes
        .route("/email/templates", get(email::list_templates).post(email::create_template))
        .route("/email/templates/:id", get(email::get_template).put(email::update_template).delete(email::delete_template))
        .route("/email/campaigns", get(email::list_campaigns).post(email::create_campaign))
        .route("/email/campaigns/:id", get(email::get_campaign).put(email::update_campaign).delete(email::delete_campaign))
        .route("/email/campaigns/:id/send", post(email::send_campaign))
        // ??? Public / landing page routes
        .route("/public/homepage", get(public::homepage_data))
        // Dynamic OG image SVG generation (MUST come before :slug routes)
        .route("/public/og/:page_type/:page_id", get(dynamic_og::dynamic_og_image))
        // ??? Onboarding survey public endpoints (MUST come before :slug routes)
        .route("/public/directories/:slug/survey/respond", post(onboarding_survey::public_submit_survey))
        .route("/public/directories/:slug/survey", get(onboarding_survey::public_get_survey))
        // ? Public articles XML feed (RSS) (MUST come before :slug routes)
        .route("/public/directories/:slug/articles.xml", get(articles_feed::articles_xml_feed))
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
        .route("/subscriptions/plans", get(monetization::list_plans))
        .route("/subscriptions/upgrade", post(monetization::upgrade_subscription))
        .route("/subscriptions/downgrade", post(monetization::downgrade_subscription))
        .route("/subscriptions/features", get(monetization::check_feature_access))
        .route("/businesses/:id/subscription", get(monetization::business_subscription))
        .route("/businesses/:id/categories", get(monetization::list_business_categories).post(monetization::update_business_categories))
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
        .route("/n8n/health", get(automation::n8n_health))
        .route("/available-providers", get(provider_keys_handler::list_available_providers))
        .route("/webhooks/stripe", post(checkout_handler::stripe_webhook))
        .route("/webhooks/paypal", post(checkout_handler::paypal_webhook))
        // Data Pipeline (BL20) — public ingress endpoint
        .route("/pipeline/ingest", post(pipeline::pipeline_ingest))
        // ??? Public industry listing (for signup forms)
        .route("/industries/available", get(industries::list_available_industries))
        // � Directory SEO & Content Infrastructure
        .route("/directories/:id/services", get(services_locations::list_services).post(services_locations::create_service))
        .route("/directories/:id/services/:svc_id", put(services_locations::update_service).delete(services_locations::delete_service))
        .route("/directories/:id/locations", get(services_locations::list_locations).post(services_locations::create_location))
        .route("/directories/:id/locations/:loc_id", put(services_locations::update_location).delete(services_locations::delete_location))
        .route("/directories/:id/services/import", post(services_locations::csv_import))
        .route("/directories/:id/locations/import", post(services_locations::csv_import))
        .route("/directories/:id/programmatic-pages", get(content_seo::list_programmatic_pages))
        .route("/directories/:id/programmatic-pages/:page_id", get(content_seo::get_programmatic_page))
        .route("/directories/:id/programmatic-pages/generate", post(content_seo::generate_programmatic_pages))
        .route("/directories/:id/programmatic-pages/bulk-status", post(content_seo::bulk_update_page_status))
        .route("/directories/:id/topics", get(content_seo::list_topics).post(content_seo::create_topic))
        .route("/directories/:id/topics/:topic_id", put(content_seo::update_topic).delete(content_seo::delete_topic))
        .route("/directories/:id/topics/bulk", post(content_seo::bulk_topic_action))
        .route("/directories/:id/topics/suggestions", get(content_seo::suggest_topics))
        .route("/directories/:id/authors", get(content_seo::list_authors).post(content_seo::create_author))
        .route("/directories/:id/authors/:author_id", put(content_seo::update_author).delete(content_seo::delete_author))
        .route("/directories/:id/ai-draft", post(content_seo::generate_ai_draft))
        .route("/directories/:id/repurpose", post(content_seo::repurpose_content))
        .route("/directories/:id/internal-links", get(content_seo::internal_link_suggestions))
        .route("/directories/:id/seo-fallbacks", get(seo_config::list_seo_fallbacks))
        .route("/directories/:id/seo-fallbacks/:page_type", put(seo_config::upsert_seo_fallback))
        .route("/directories/:id/schema-configs", get(seo_config::list_schema_configs))
        .route("/directories/:id/schema-configs/:schema_type", put(seo_config::upsert_schema_config))
        .route("/directories/:id/seo-settings", get(seo_config::get_dir_seo_settings).put(seo_config::update_dir_seo_settings))
        .route("/directories/:id/sitemap", get(seo_config::generate_sitemap))
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
        // ??? Provider keys management
        .route("/provider-keys", get(provider_keys_handler::list_provider_keys).post(provider_keys_handler::upsert_provider_key))
        .route("/provider-keys/:provider", put(provider_keys_handler::upsert_provider_key).delete(provider_keys_handler::delete_provider_key))
        .route("/provider-keys/:provider/test", get(provider_keys_handler::test_provider_key))
        // ??? Payment provider management
        .route("/payment-providers", get(checkout_handler::list_payment_providers).post(checkout_handler::upsert_payment_provider))
        .route("/payment-providers/{provider_type}", delete(checkout_handler::delete_payment_provider))
        .route("/checkout/create", post(checkout_handler::create_checkout_session))
        .route("/checkout/sessions", get(checkout_handler::list_checkout_sessions))
        // ??? Industry dashboard routes
        .route("/industries", get(industries::list_user_industries).post(industries::set_user_industry))
        .route("/industries/:slug", delete(industries::remove_user_industry))
        .route("/industries/limit", get(industries::get_industry_limit))
        // ? Portal routes (business_owner auth)
        .route("/portal/business/profile", get(portal::business_profile))
        .route("/portal/business/dashboard", get(business_dashboard::business_dashboard))
        // ? Visitor account routes
        .route("/visitor/register", post(portal::visitor_register))
        .route("/visitor/login", post(portal::visitor_login))
        .route("/visitor/profile", get(portal::visitor_profile))
        // ? Directory feature config (public GET, admin PUT)
        .route("/directories/:id/features", get(portal::get_directory_features).put(portal::update_directory_features))
        // ? Public endpoints (no auth required)
        .route("/businesses/:id/claim", post(visitors::claim_business))
        .route("/businesses/:id/images", post(businesses::upload_business_images))
        // ? Booking routes (no auth required)
        .route("/directories/:slug/businesses/:business_id/available-slots", get(bookings::get_available_slots))
        .route("/directories/:slug/businesses/:business_id/book", post(bookings::create_booking))
        // ? Public booking page (no auth required, also outside the auth middleware)
        .route("/book/:slug/:business_id", get(booking_page::booking_page))
        // ? BL29: Pricing engine — admin routes
        .route("/pricing/services", get(pricing::list_services))
        .route("/pricing/services/:service_key", put(pricing::update_service_price))
        .route("/pricing/bundles", get(pricing::list_bundles).post(pricing::create_bundle))
        .route("/pricing/bundles/:id", get(pricing::get_bundle).put(pricing::update_bundle).delete(pricing::delete_bundle))
        .route("/pricing/grandfather", post(pricing::set_grandfathered))
        .route("/pricing/grandfather/:business_id", get(pricing::get_grandfathered))
        // ? BL29: Pricing engine — public endpoint (no auth)
        .route("/pricing/public", get(pricing::public_pricing))
        // ??? Contact Intelligence Pipeline — monthly cron for unclaimed business enrichment
        .route("/cron/contact-intelligence", post(contact_intelligence::contact_intelligence_pipeline))
        // ??? Content Queue routes (Phase 5 Task 3)
        .route("/admin/content-queue", get(content_queue::list_queue).post(content_queue::add_job))
        .route("/admin/content-queue/:id", put(content_queue::update_job).delete(content_queue::cancel_job))
        .route("/admin/content-queue/bulk", post(content_queue::bulk_add_jobs))
        .route("/cron/content-queue-worker", post(content_queue::process_content_queue))
        // ??? Tag Automation + Tracked Links (Task 4)
        .route("/admin/tag-rules", get(tag_automation::list_rules).post(tag_automation::create_rule))
        .route("/admin/tag-rules/:id", put(tag_automation::update_rule).delete(tag_automation::delete_rule))
        .route("/admin/tag-rules/execute", post(tag_automation::execute_rules_for_contact))
        .route("/admin/tracked-links", get(tag_automation::list_tracked_links).post(tag_automation::create_tracked_link))
        .route("/admin/tracked-links/:id", put(tag_automation::update_tracked_link).delete(tag_automation::delete_tracked_link))
        .route("/admin/tracked-links/bulk", post(tag_automation::bulk_create_tracked_links))
        .route("/admin/tracked-links/stats/:id", get(tag_automation::get_link_stats))
        // ??? Onboarding Survey admin endpoints
        .route("/admin/directories/:id/survey", get(onboarding_survey::get_survey_config).put(onboarding_survey::upsert_survey_config))
        .route("/admin/directories/:id/survey/toggle", post(onboarding_survey::toggle_survey))
        // ??? Cross-platform tag sync
        .route("/admin/tag-sync", post(tag_sync::sync_tag_across_platforms))
        .layer(middleware::from_fn_with_state(
            s.clone(),
            auth_guard,
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

    // Clone state for host resolution before it enters move closures
    let _state_for_host = s.clone();
    let _config_for_host = s.config.clone();

    // ??? Combine: /api/v1/* API routes + static file server at /* + SPA fallback
    let app = Router::new()
        .route("/l/:short_code", get(tag_automation::track_link_click))
        // Dynamic OG images also available at root level (mirrors /api/v1/public/og/...)
        .route("/public/og/:page_type/:page_id", get(dynamic_og::dynamic_og_image))
        .nest("/api/v1", all_routes)
        .fallback_service(
            tower::service_fn(move |req: axum::http::Request<axum::body::Body>| {
                let frontend = frontend_path.clone();
                let index_clone = index_content.clone();
                let login_clone = login_content.clone();
                let index_clone2 = index_content2.clone();
                let _pool_for_host = _state_for_host.db.clone();
                let _base_domain = _config_for_host.base_domain.clone();
                async move {
                    // ── Host-based directory resolution ──
                    // Check if Host header matches a registered domain mapping
                    let host = req.headers().get("Host")
                        .and_then(|v| v.to_str().ok())
                        .map(|h| h.trim().to_lowercase());

                    let path = req.uri().path().to_string();

                    if let Some(ref host) = host {
                        let app_domain = _base_domain.to_lowercase();
                        let www_domain = format!("www.{}", app_domain);
                        let is_app = host == &app_domain
                            || host == &www_domain
                            || host == "localhost"
                            || host.starts_with("127.0.0.1")
                            || host.starts_with("192.168.")
                            || host.starts_with("10.");

                        if !is_app && !path.starts_with("/api/") && !path.starts_with("/admin") && path != "/health" {
                            let domain = host.split(':').next().unwrap_or(host).to_string();

                            let result = sqlx::query_as::<_, (uuid::Uuid, String)>(
                                r#"SELECT dm.directory_id, d.slug
                                   FROM domain_mappings dm
                                   JOIN directories d ON d.id = dm.directory_id
                                   WHERE dm.domain = $1 AND dm.status = 'active'
                                   LIMIT 1"#
                            )
                            .bind(&domain)
                            .fetch_optional(&_pool_for_host)
                            .await;

                            if let Ok(Some((_dir_id, slug))) = result {
                                if !path.starts_with(&format!("/d/{}", slug)) {
                                    let pq = req.uri().path_and_query()
                                        .map(|pq| pq.as_str())
                                        .unwrap_or("/");

                                    let new_path = if pq == "/" {
                                        format!("/d/{}", slug)
                                    } else {
                                        format!("/d/{}{}", slug, pq)
                                    };

                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::FOUND)
                                            .header("Location", &new_path)
                                            .body(axum::body::Body::empty())
                                            .unwrap()
                                    );
                                }
                            }
                        }
                    }
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

                    // ??? BL29: Serve pricing admin page
                    if path == "/admin/pricing" || path.starts_with("/admin/pricing/") {
                        let pricing_path = std::path::Path::new(&frontend).join("pricing-admin.html");
                        if pricing_path.exists() {
                            match tokio::fs::read(&pricing_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }

                    // ??? Serve portal pages by redirecting to the HTML file (fallback file serve handles it)
                    if path == "/portal" || path == "/portal/" || path.starts_with("/portal/business") {
                        let portal_path = std::path::Path::new(&frontend).join("business-portal.html");
                        if portal_path.exists() {
                            match tokio::fs::read(&portal_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    if path == "/visitor" || path == "/visitor/" || path.starts_with("/visitor/portal") {
                        let portal_path = std::path::Path::new(&frontend).join("visitor-portal.html");
                        if portal_path.exists() {
                            match tokio::fs::read(&portal_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }

                    // ??? Serve hotel-savings FAQ and Terms pages
                    if path == "/hotel-savings/faq" {
                        let faq_path = std::path::Path::new(&frontend).join("hotel-savings-faq.html");
                        if faq_path.exists() {
                            match tokio::fs::read(&faq_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    if path == "/hotel-savings/terms" {
                        let terms_path = std::path::Path::new(&frontend).join("hotel-savings-terms.html");
                        if terms_path.exists() {
                            match tokio::fs::read(&terms_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }

                    // ??? Serve distributor/B2B supplier portal
                    if path == "/distributor" || path == "/distributor/" || path.starts_with("/distributor/dashboard") {
                        let portal_path = std::path::Path::new(&frontend).join("distributor-portal.html");
                        if portal_path.exists() {
                            match tokio::fs::read(&portal_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
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


/// Auth guard middleware — requires JWT on all routes except public ones
async fn auth_guard(
    State(s): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let path = req.uri().path().to_string();
    let has_auth = req.headers().get("Authorization").is_some();
    let auth_preview = req.headers().get("Authorization").and_then(|v| v.to_str().ok()).unwrap_or("none").chars().take(40).collect::<String>();
    warn!("AUTH_GUARD: path={}, has_auth={}, auth_preview={}", path, has_auth, auth_preview);
    
    // Public paths that don't need authentication
    let is_public = path == "/health"
        || path == "/auth/login"
        || path == "/auth/register"
        || path == "/auth/forgot-password"
        || path == "/auth/reset-password"
        || path.starts_with("/sitemap.xml")
        || path.starts_with("/robots.txt")
        || path.starts_with("/public/")
        || path.starts_with("/api/v1/public/")
        || path == "/categories"
        || path == "/search"
        || path == "/listings"
        || path.starts_with("/reviews/stats/")
        // Public newsletter signup — no auth needed
        || (path.contains("/subscribers") && req.method() == "POST")
        // Public directory search suggestions
        || path.ends_with("/suggestions")
        // Public visitor account routes
        || path == "/visitor/register"
        || path == "/visitor/login"
        // Public B2B register (distributor/supplier signup)
        || (path == "/b2b/register" && req.method() == "POST")
        // Public pricing endpoint
        || path == "/pricing/public"
        // Public data pipeline ingest (external sources push here)
        || path == "/pipeline/ingest"
        // Public community posts (GET only, POST/PUT/DELETE need auth)
        || (path == "/community/posts" && req.method() == "GET")
        || (path.starts_with("/community/posts/") && req.method() == "GET")
        // Public B2B marketplace (read-only, POST/PUT/DELETE need auth)
        || (path == "/b2b/products" && req.method() == "GET")
        || (path.starts_with("/b2b/products/") && req.method() == "GET")
        || path == "/b2b/suppliers"
        // Public scraper provider list (read-only)
        || path == "/scraper/providers"
        // Public provider key test
        || (path.starts_with("/provider-keys/") && path.ends_with("/test"))
        // Public subscription plans + features
        || path == "/subscriptions/plans"
        || path == "/subscriptions/features"
        // Public scraper provider list (read-only)
        // Public deal redemption (visitors redeem codes without auth)
        || (path.starts_with("/deals/") && path.ends_with("/redeem") && req.method() == "POST")
        || (path.starts_with("/deals/redemptions/code/") && req.method() == "GET")
        // Public featured deals
        || (path.ends_with("/features") && req.method() == "GET")
        // Public business claim form
        || (path.starts_with("/businesses/") && path.ends_with("/claim") && req.method() == "POST")
        // Cron endpoints (triggered by cron daemon with optional API key)
        || (path == "/cron/contact-intelligence" && req.method() == "POST")
        || (path == "/cron/content-queue-worker" && req.method() == "POST")
        // Public business image upload
        || (path.starts_with("/businesses/") && path.ends_with("/images") && req.method() == "POST")
        // Public booking endpoints
        || (path.contains("/available-slots") && req.method() == "GET")
        || (path.contains("/book") && req.method() == "POST" && !path.contains("blog"))
        // Public booking page (GET)
        || (path.starts_with("/book/") && req.method() == "GET");
    
    if is_public {
        return Ok(next.run(req).await);
    }
    
    // For all other routes, require valid JWT
    use crate::auth::middleware::verify_token;
    
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;
    
    let token = auth_header.strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;
    
    let claims = verify_token(token, &s.config.jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;
    
    // Insert claims into request extensions for handlers that need them
    req.extensions_mut().insert(claims);
    
    Ok(next.run(req).await)
}


/// GET /api/v1/health
async fn health_check() -> impl axum::response::IntoResponse {
    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({
        "status": "ok",
        "service": "multidirectory-api",
        "version": env!("CARGO_PKG_VERSION")
    })))
}

