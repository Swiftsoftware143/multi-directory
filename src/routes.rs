use axum::{
    routing::{get, post, put, delete, patch},
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
        // ? SSO Role Switcher (Stage 5)
        .route("/auth/switch-role", post(sso::switch_role))
        .route("/auth/linked-accounts", get(sso::get_linked_accounts))
        .route("/directories", get(directories::list_directories).post(directories::create_directory))
        .route("/directories/:slug", get(directories::get_directory).put(directories::update_directory).delete(directories::delete_directory))
        // Legacy /directory/:slug alias (used by older frontend code)
        .route("/directory/:slug", get(directories::get_directory))
        .route("/directories/:slug/render", get(directories::render_directory))
        .route("/directories/:slug/categories", get(directories::list_categories).post(directories::create_category))
        .route("/directories/:slug/categories/:category_id", put(directories::update_category).delete(directories::delete_category))
        .route("/directories/:slug/businesses", get(businesses::list_businesses).post(businesses::create_business))
        .route("/directories/:slug/businesses/suggestions", get(businesses::search_suggestions))
        // Legacy /directory/:slug/businesses alias
        .route("/directory/:slug/businesses", get(businesses::list_businesses))
        .route("/directories/:slug/businesses/:business_id", get(businesses::get_business).put(businesses::update_business).delete(businesses::delete_business))
        .route("/directories/:slug/loyalty/programs", get(loyalty_native::list_programs).post(loyalty_native::create_program))
        .route("/directories/:slug/loyalty/programs/:program_id", get(loyalty_native::get_program_handler).put(loyalty_native::update_program).delete(loyalty_native::delete_program))
        .route("/directories/:slug/loyalty/programs/:program_id/enroll", post(loyalty_native::enroll_member))
        .route("/directories/:slug/loyalty/programs/:program_id/checkin", post(loyalty_native::checkin))
        .route("/directories/:slug/loyalty/members/:visitor_account_id", get(loyalty_native::get_member))

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
        .route("/directories/newsletter", post(newsletter::add_global_subscriber))
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
        // Blog Features (content decay, internal linking, AEO scoring, schema markup)
        .route("/blog/decay/scan", post(blog_features::scan_content_decay))
        .route("/blog/decay/refresh/:id", post(blog_features::refresh_post_content))
        .route("/blog/internal-links/suggestions", get(blog_features::internal_link_suggestions))
        .route("/blog/internal-links/add", post(blog_features::add_internal_link))
        .route("/blog/internal-links/:post_id/:target_id", delete(blog_features::remove_internal_link))
        .route("/blog/aeo/score/:id", post(blog_features::score_post_aeo))
        .route("/blog/aeo/scan-all", post(blog_features::score_all_posts_aeo))
        .route("/blog/aeo/report", get(blog_features::aeo_report))
        .route("/blog/schema/generate/:id", post(blog_features::generate_schema_markup))
        .route("/blog/schema/generate-all", post(blog_features::generate_all_schema))

        // Content Research & Strategy Engine
        .route("/research/topics", get(content_research::list_topics).post(content_research::create_topic))
        .route("/research/topics/:id", put(content_research::update_topic).delete(content_research::delete_topic))
        .route("/research/run", post(content_research::research_topic))
        .route("/research/questions", get(content_research::get_research_questions))
        .route("/research/questions/:id/use-as-keyword", post(content_research::use_question_as_keyword))
        .route("/research/draft-post", post(content_research::draft_post_from_question))
        .route("/research/bulk", post(content_research::bulk_research))
        .route("/research/bulk/preview", post(content_research::bulk_research_preview))
        .route("/research/integrations", get(content_research::list_integrations).post(content_research::save_integration))
        .route("/research/integrations/:provider", delete(content_research::delete_integration))

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
        .route("/deals/:id/page", get(deals::get_deal_page))
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
        .route("/api/v1/search", get(blog_qa::search_all))
        .route("/cities", get(zaarhub_cities::list_cities))
        .route("/categories", get(categories::list_all_categories))
        // Public aliases for frontend
        .route("/stats/public", get(zaarhub::get_homepage))
        .route("/subscription-plans", get(monetization::list_plans))
        // Phase 2: Multi-category system
        .route("/categories/filter-options", get(category_system::get_filter_options))
        .route("/businesses/:id/categories", get(category_system::get_business_categories).put(category_system::set_business_categories).post(category_system::set_business_categories))
        .route("/businesses/:id/categories/:category_id", delete(category_system::delete_business_category))
        .route("/businesses/bulk/categories", post(category_system::bulk_assign_categories))
        .route("/businesses/:id/category-requests", post(category_system::create_category_request))
        .route("/category-requests", get(category_system::list_category_requests))
        .route("/category-requests/:id/approve", post(category_system::approve_category_request))
        .route("/category-requests/:id/deny", post(category_system::deny_category_request))
        // Community posts (BL27)
        .route("/community/posts", get(blog::list_community_posts).post(blog::create_community_post))
        .route("/community/posts/:id", get(blog::get_blog_post).put(blog::update_community_post).delete(blog::delete_blog_post))
        // Q&A Automation
        .route("/api/v1/blog-qa/fetch-keywords", post(blog_qa::fetch_keywords))
        .route("/api/v1/blog-qa/generate-posts", post(blog_qa::generate_posts))
        .route("/api/v1/blog-qa/keywords", get(blog_qa::list_keywords))
        .route("/api/v1/blog-qa/generate-digest", post(blog_qa::generate_digest))
        .route("/api/v1/blog-qa/send-digest", post(blog_qa::send_digest))
        .route("/api/v1/blog-qa/schedule-weekly", post(blog_qa::schedule_weekly))
        .route("/api/v1/integration-configs", get(blog_qa::list_configs).post(blog_qa::save_config))
        .route("/api/v1/integration-configs/:provider", get(blog_qa::get_config).delete(blog_qa::delete_config))
        // ??? Answer-First article generator + CoreSwift tenant setup
        .route("/api/v1/admin/articles/generate-answer-first", post(answer_first::generate_answer_first))
        .route("/api/v1/admin/articles/suggest-competitors", post(answer_first::suggest_competitors))
        .route("/api/v1/admin/core-swift/setup-tenant", post(answer_first::setup_core_swift_tenant))
        .route("/api/v1/admin/core-swift/test-connection", post(answer_first::test_core_swift_connection))
        // RFQ Marketplace (BL24)
        .route("/b2b/rfqs/stats", get(rfq::rfq_stats))
        .route("/b2b/rfqs/my", get(rfq::my_rfqs))
        .route("/b2b/rfqs", get(rfq::list_rfqs).post(rfq::create_rfq))
        .route("/b2b/rfqs/:id", get(rfq::get_rfq).patch(rfq::update_rfq))
        .route("/b2b/rfqs/:id/bids", get(rfq::list_bids).post(rfq::submit_bid))
        .route("/b2b/rfqs/:id/bids/:bid_id/accept", post(rfq::accept_bid))
        .route("/b2b/rfqs/:id/bids/:bid_id/reject", post(rfq::reject_bid))
        .route("/b2b/rfqs/:id/messages", get(rfq::get_rfq_messages).post(rfq::post_rfq_message))
        // Lead Sharing (BL25)
        .route("/b2b/leads", post(lead_sharing::share_lead))
        .route("/b2b/leads/available", get(lead_sharing::available_leads))
        .route("/b2b/leads/my", get(lead_sharing::my_leads))
        .route("/b2b/leads/:id/claim", post(lead_sharing::claim_lead))
        // Co-op Buying Groups (BL26)
        .route("/b2b/co-op/groups", get(coop::list_groups).post(coop::create_group))
        .route("/b2b/co-op/groups/:id", get(coop::get_group))
        .route("/b2b/co-op/groups/:id/join", post(coop::join_group))
        .route("/b2b/co-op/groups/:id/deals", post(coop::create_deal))
        .route("/b2b/co-op/my-groups", get(coop::my_groups))
        .route("/b2b/co-op/deals/active", get(coop::active_deals))
        .route("/b2b/co-op/deals/:id/commit", post(coop::commit_to_deal))
        // B2B Marketplace (Phase 4 — BL23)
        .route("/b2b/register", post(b2b::b2b_register))
        .route("/b2b/products", get(b2b::search_products).post(b2b::create_product))
        .route("/b2b/products/my", get(b2b::my_products))
        .route("/b2b/products/export", get(b2b::export_products))
        .route("/b2b/products/import", post(b2b::import_products))
        .route("/b2b/products/:id", get(b2b::get_product).put(b2b::update_product).delete(b2b::delete_product))
        .route("/b2b/suppliers", get(b2b::list_suppliers))
        .route("/b2b/orders", post(b2b::place_order).get(b2b::my_orders))
        .route("/b2b/orders/:id", get(b2b::get_order))
        .route("/b2b/orders/:id/status", put(b2b::update_order_status))
        .route("/b2b/messages", post(b2b::send_b2b_message).get(b2b::my_b2b_messages))
        .route("/b2b/messages/:id/read", put(b2b::mark_message_read))
        // B2B Marketplace (Phase 3b — public)
        .route("/b2b/marketplace", get(b2b::marketplace))
        .route("/b2b/suppliers/:id/detail", get(b2b::supplier_detail))
        // B2B Supplier Discovery (Phase 3d — public)
        .route("/b2b/discover", get(b2b::discover_suppliers))
        // B2B Notifications (Phase 3e)
        .route("/b2b/notifications", get(b2b::my_notifications))
        .route("/b2b/notifications/:id/read", put(b2b::mark_notification_read))
        .route("/b2b/notifications/read-all", put(b2b::mark_all_read))
        // Supplier Portal (back office)
        .route("/supplier/profile", get(supplier::get_supplier_profile).put(supplier::update_supplier_profile))
        .route("/supplier/delivery", put(supplier::update_delivery_settings))
        .route("/supplier/featured-product", put(supplier::set_featured_product))
        .route("/supplier/stats", get(supplier::supplier_stats))
        .route("/b2b/orders/:id/fulfill", put(supplier::fulfill_order))
        .route("/b2b/orders/:id/review", post(supplier::supplier_review_buyer))
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
        .route("/d/:slug/blog", get(blog_pages::render_blog_list))
        .route("/d/:slug/blog/:post_slug", get(blog_pages::render_blog_post))
        .route("/public/homepage", get(public::homepage_data))
        // Dynamic OG image SVG generation (MUST come before :slug routes)
        .route("/public/og/:page_type/:page_id", get(dynamic_og::dynamic_og_image))
        // ??? Onboarding survey public endpoints (MUST come before :slug routes)
        .route("/public/directories/:slug/survey/respond", post(onboarding_survey::public_submit_survey))
        .route("/public/directories/:slug/survey", get(onboarding_survey::public_get_survey))
        // ? Public articles XML feed (RSS) (MUST come before :slug routes)
        .route("/public/directories/:slug/articles.xml", get(articles_feed::articles_xml_feed))
        .route("/public/directories/:slug/news-sitemap.xml", get(blog_seo::news_sitemap))
        .route("/public/directories/:slug/blog/feed.xml", get(blog_seo::blog_rss_feed))
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
        // (moved to category_system above)
        // ? Business self-serve ad submission (auth required)
        .route("/businesses/:business_id/ads/submit", post(monetization::submit_business_ad))
        .route("/businesses/:business_id/ads", get(monetization::list_business_ads))
        .route("/businesses/:business_id/ads/earnings", get(monetization::get_business_ad_earnings))
        .route("/ad-zones", get(monetization::list_ad_zones).post(monetization::create_ad_zone))
        .route("/ad-zones/:id", get(monetization::get_ad_zone).put(monetization::update_ad_zone).delete(monetization::delete_ad_zone))
        // &#128176; Sponsor & Ad Management (Phase 4)
        .route("/sponsors", get(monetization::list_sponsors).post(monetization::create_sponsor))
        .route("/sponsors/:id", put(monetization::update_sponsor).delete(monetization::delete_sponsor))
        .route("/sponsors/:id/creatives", get(monetization::list_creatives).post(monetization::create_creative))
        .route("/creatives/:id", put(monetization::update_creative).delete(monetization::delete_creative))
        .route("/schedules", get(monetization::list_schedules).post(monetization::create_schedule))
        .route("/schedules/:id", put(monetization::update_schedule).delete(monetization::delete_schedule))
        // .route("/ads/active/:directory_id", get(monetization::get_active_ads)) -- moved to public
        .route("/earnings/:directory_id", get(monetization::get_earnings_summary))
        .route("/approvals", get(monetization::list_approvals))
        .route("/approvals/:id/status", put(monetization::update_approval))
        .route("/directories/:slug/ad-zones", get(monetization::directory_ad_zones))
        // ??? Directory tier routes (Phase 3 Task 3)
        .route("/monetization/directory-tiers", get(monetization::list_directory_tiers).post(monetization::create_directory_tier))
        .route("/monetization/directory-tiers/:id", get(monetization::get_directory_tier).put(monetization::update_directory_tier).delete(monetization::delete_directory_tier))
        .route("/monetization/directories/:slug/tier", get(monetization::directory_tier_by_slug))
        // ??? Sponsored listing routes (Phase 3 Task 3)
        .route("/monetization/sponsored-listings", get(monetization::list_sponsored_listings).post(monetization::create_sponsored_listing))
        .route("/monetization/sponsored-listings/:id", get(monetization::get_sponsored_listing).put(monetization::update_sponsored_listing).delete(monetization::delete_sponsored_listing))
        .route("/monetization/directories/:slug/sponsored-listings", get(monetization::directory_sponsored_listings))
        // ??? Directory Notification routes (Phase 4)
        .route("/monetization/notifications", get(monetization::list_notifications).post(monetization::create_notification))
        .route("/monetization/notifications/:id", put(monetization::update_notification).delete(monetization::delete_notification))
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
        .route("/directory-events", get(automation::list_events).post(automation::create_event))
        .route("/directory-events/unprocessed", get(automation::unprocessed_events))
        .route("/directory-events/:id/process", post(automation::mark_event_processed))
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
        // ??? Trap Door hyper-niche SEO pages
        .route("/directories/:id/trap-doors/generate", post(trap_doors::generate_trap_doors))
        .route("/directories/:id/trap-doors/preview", post(trap_doors::preview_trap_doors))
        .route("/directories/:id/trap-doors/available-factors", get(trap_doors::available_factors))
        .route("/directories/:id/trap-doors/pages", get(trap_doors::list_trap_door_pages))
        .route("/directories/:id/trap-doors/analytics", get(trap_doors::trap_door_analytics))
        .route("/directories/:id/trap-doors/scheduled-generate", post(trap_doors::scheduled_generate_trap_doors))
        .route("/programmatic-pages/:page_id/track", post(trap_doors::track_page_event))
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
        .route("/branding/:directory_id/upload", post(branding::upload_branding_asset))
        .route("/branding/:directory_id/extract", post(branding::extract_colors))
        .route("/members", get(admin::admin_members))
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
        .route("/payment-providers/:provider_type", delete(checkout_handler::delete_payment_provider))
        .route("/checkout/create", post(checkout_handler::create_checkout_session))
        .route("/checkout/sessions", get(checkout_handler::list_checkout_sessions))
        // ??? Industry dashboard routes
        .route("/industries", get(industries::list_user_industries).post(industries::set_user_industry))
        .route("/industries/:slug", delete(industries::remove_user_industry))
        .route("/industries/limit", get(industries::get_industry_limit))
        // ? Visitor account routes (no auth — self-contained)
        .route("/visitor/register", post(portal::visitor_register))
        .route("/visitor/login", post(portal::visitor_login))
        .route("/visitor/profile", get(portal::visitor_profile))
        .route("/visitor/favorites", get(visitors::list_favorites))
        .route("/visitor/favorites/check/:business_id", get(visitors::check_favorite))
        .route("/visitor/favorites/:business_id", post(visitors::toggle_favorite))
        // ? Visitor bookmarks / saved places — alternate scoped routes
        .route("/bookmarks", get(visitors::list_favorites))
        .route("/bookmarks/toggle", post(visitors::toggle_bookmark))
        .route("/bookmarks/count/:business_id", get(visitors::get_bookmark_count))
        // ? Micro-Polls (v2: public GET, visitor/require auth for vote/create/close)
        .route("/polls", get(polls::list_polls).post(polls::create_poll))
        .route("/polls/:id", get(polls::get_poll))
        .route("/polls/:id/vote", post(polls::cast_vote))
        .route("/polls/:id/close", post(polls::close_poll))
        // ? Community Events with RSVP (Stage 3)
        .route("/events", get(events::list_events).post(events::create_event))
        .route("/events/:id", get(events::get_event))
        .route("/events/:id/rsvp", post(events::rsvp_event))
        .route("/events/:id/attendees", get(events::list_attendees))
        .route("/events/:id/cancel", post(events::cancel_event))
        .route("/events/:id/edit", post(events::edit_event))
        .route("/events-page", get(events::events_page))
        // Neighborhood Feed routes (Stage 4)
        .route("/feed", get(feed::get_feed))
        .route("/feed-page", get(feed::feed_page))
        // Server-rendered saved places page (auth via auth_guard)
        .route("/saved-places", get(public::saved_places_page))
        // ? Portal + Loyalty routes (authenticated — JWT required)
        .route("/portal/business/profile", get(portal::business_profile))
        .route("/portal/business/dashboard", get(business_dashboard::business_dashboard))
        .route("/loyalty/pin/status", get(loyalty_proxy::pin_status))
        .route("/loyalty/pin/generate", post(loyalty_proxy::pin_generate))
        .route("/loyalty/pin/verify", post(loyalty_proxy::pin_verify))
        .route("/loyalty/credits/balance", get(loyalty_proxy::credits_balance))
        .route("/loyalty/credits/history", get(loyalty_proxy::credits_history))
        .route("/loyalty/vouchers", get(loyalty_proxy::vouchers_list))
        .route("/loyalty/vouchers/redeem", post(loyalty_proxy::voucher_redeem))
        .route("/loyalty/referrals", get(loyalty_proxy::referrals_list))
        .route("/loyalty/referrals/create", post(loyalty_proxy::referral_create))
        .route("/loyalty/rewards", get(loyalty_proxy::rewards_list))
        .route("/loyalty/rewards/claim", post(loyalty_proxy::reward_claim))
        .route("/loyalty/pledges", get(loyalty_proxy::pledges_list))
        .route("/loyalty/pledges/create", post(loyalty_proxy::pledge_create))
        .route("/loyalty/enroll", post(loyalty_proxy::enroll))
        .route("/loyalty/portal/dashboard", get(loyalty_proxy::portal_dashboard))
        .route("/loyalty/qr", get(loyalty_proxy::get_loyalty_qr))
        .route("/loyalty/purchase/verify", post(loyalty_proxy::purchase_verify_proxy))
        .route("/loyalty/admin/credit-rate", get(loyalty_proxy::get_credit_rate).patch(loyalty_proxy::update_credit_rate))
        .route("/loyalty/admin/purchase-pin", get(loyalty_proxy::get_purchase_pin))
        .route("/loyalty/admin/offers", get(loyalty_proxy::offers_list).post(loyalty_proxy::offers_create))
        .route("/loyalty/admin/offers/:id", get(loyalty_proxy::offers_get).put(loyalty_proxy::offers_update).delete(loyalty_proxy::offers_delete))
        .route("/business/loyalty/status", get(loyalty_subscription::loyalty_status))
        .route("/business/loyalty/subscribe", post(loyalty_subscription::loyalty_subscribe))
        // Campaigns Proxy Route (IS campaigns for dropdown picker)
        .route("/campaigns/list", get(iqs_proxy::list_campaigns))
        // IQS Proxy Routes (proxied to IncentiveSwift)
        .route("/iqs/funnels", get(iqs_proxy::list_funnels).post(iqs_proxy::create_funnel))
        .route("/iqs/funnels/:id", get(iqs_proxy::get_funnel).put(iqs_proxy::update_funnel).delete(iqs_proxy::delete_funnel))
        .route("/iqs/funnels/:id/play", get(iqs_proxy::get_play_funnel))
        .route("/iqs/funnels/:id/submit", post(iqs_proxy::submit_funnel))
        .route("/iqs/funnels/:id/questions", get(iqs_proxy::list_questions).post(iqs_proxy::create_question))
        .route("/iqs/funnels/:id/questions/:question_id", put(iqs_proxy::update_question).delete(iqs_proxy::delete_question))
        .route("/iqs/funnels/:id/submissions", get(iqs_proxy::list_submissions))
        // Loyalty Badges, QR, Scanner Proxy Routes
        .route("/loyalty/badge/member/:member_id", get(iqs_proxy::badge_member_proxy))
        .route("/loyalty/badge/business/:business_id", get(iqs_proxy::badge_business_proxy))
        .route("/loyalty/member/:member_id/qr", get(iqs_proxy::member_qr_proxy))
        .route("/loyalty/scan", post(iqs_proxy::scan_proxy))
        .route("/loyalty/scans/business/:business_id", get(iqs_proxy::business_scans_proxy))
        .route("/loyalty/dashboard/member/:member_id", get(iqs_proxy::member_dashboard_proxy))
        .route("/loyalty/programs", get(iqs_proxy::loyalty_programs_proxy))
        .route("/loyalty/tiers", get(iqs_proxy::loyalty_tiers_proxy))
        .route("/loyalty/member/:member_id", get(iqs_proxy::loyalty_member_proxy))
        // Connected Services — API key integration for IS and CoreSwift
        .route("/connected-services", get(connected_services::list_connected_services))
        .route("/connected-services/connect", post(connected_services::connect_service))
        .route("/connected-services/verify", post(connected_services::verify_service_key))
        .route("/connected-services/coreswift/check", get(connected_services::check_coreswift_connection))
        .route("/connected-services/:service", delete(connected_services::disconnect_service))
        .route("/connected-services/:service/campaigns", get(connected_services::list_service_campaigns))
        // ??? Event Provider Pipeline (Phase 2A)
        .route("/admin/directories/:directory_id/event-providers", get(event_providers::list_providers).post(event_providers::create_provider))
        .route("/admin/directories/:directory_id/event-providers/:provider_id", put(event_providers::update_provider).delete(event_providers::delete_provider))
        .route("/admin/event-providers/:provider_id/test", post(event_providers::test_provider))
        .route("/admin/event-providers/:provider_id/sync", post(event_providers::sync_provider))
        .route("/admin/event-providers/:provider_id/sync-status", get(event_providers::sync_status))
        .layer(middleware::from_fn_with_state(
            s.clone(),
            auth_guard,
        ))
        // ? Directory feature config (public GET, admin PUT)
        .route("/directories/:id/features", get(portal::get_directory_features).put(portal::update_directory_features))
        // ? Public endpoints (no auth required)
        .route("/messages/:business_id", post(messaging::send_message))
        .route("/businesses/:id/claim", post(visitors::claim_business))
        .route("/businesses/:id/images", post(businesses::upload_business_images))
        .route("/city-requests", get(visitors::get_city_requests).post(visitors::request_city))
        // ??? ZaarHub community frontend API (non-overlapping legacy routes, others migrated to zaarhub_cities)
        .route("/zaarhub/activity", get(zaarhub::get_activity))
        .route("/zaarhub/homepage", get(zaarhub::get_homepage))
        .route("/zaarhub/business/:slug/:id", get(zaarhub::get_business_detail))
        .route("/zaarhub/deals", get(zaarhub::list_featured_deals))
        .route("/zaarhub/events", get(zaarhub::list_featured_events))
        .route("/spotlight/:id/feature", post(zaarhub::toggle_spotlight_featured))
        // ??? ZaarHub City Pages API (Phase 4 — city_pages + business_listings CRUD)
        .route("/zaarhub/cities", get(zaarhub_cities::list_cities))
        .route("/zaarhub/cities/:slug", get(zaarhub::get_city_page))
        .route("/zaarhub/cities/:slug/listings", get(zaarhub_cities::list_city_listings))
        .route("/zaarhub/listings/:id", get(zaarhub_cities::get_listing))
        .route("/zaarhub/categories", get(zaarhub_cities::list_categories))
        .route("/zaarhub/search", get(zaarhub_cities::search_listings))
        .route("/zaarhub/featured", get(zaarhub_cities::featured_listings))
        // ??? Editor's Picks (public GET + admin PATCH)
        .route("/zaarhub/editors-picks", get(zaarhub_cities::list_editors_picks))
        .route("/zaarhub/admin/listings/:id/editors-pick", patch(zaarhub_cities::toggle_editors_pick))
        // ??? ZaarHub Claim/Redemption endpoints (public, Phase 5)
        .route("/zaarhub/listings/:id/offers", get(zaarhub_cities::listing_offers))
        .route("/zaarhub/offers/:id", get(zaarhub_cities::get_offer))
        .route("/zaarhub/offers/:id/claim", post(zaarhub_cities::claim_offer))
        .route("/zaarhub/claims/:visitor_id", get(zaarhub_cities::visitor_claims))
        // ??? ZaarHub Analytics (public read-only)
        .route("/zaarhub/analytics/overview", get(zaarhub_analytics::overview))
        .route("/zaarhub/analytics/cities", get(zaarhub_analytics::city_performance))
        .route("/zaarhub/analytics/offers", get(zaarhub_analytics::top_offers))
        .route("/zaarhub/analytics/claims", get(zaarhub_analytics::recent_claims))
        .route("/zaarhub/analytics/categories", get(zaarhub_analytics::category_breakdown))
        // ??? ZaarHub Admin (legal pages + site config)
        .route("/zaarhub/admin/legal", get(zaarhub_admin::list_legal_pages).post(zaarhub_admin::save_legal_page))
        .route("/zaarhub/admin/legal/:slug", get(zaarhub_admin::get_legal_page).delete(zaarhub_admin::delete_legal_page))
        .route("/zaarhub/admin/config", get(zaarhub_admin::get_site_config).patch(zaarhub_admin::update_site_config))
        .route("/ads/active/:directory_id", get(monetization::get_active_ads))
        // ??? Public Spotlight & Notifications endpoints (Phase 4)
        .route("/spotlight/:directory_id", get(monetization::get_spotlight_businesses))
        .route("/notifications/:directory_id", get(monetization::get_active_notifications))
        // ZaarHub config is managed via PUT /directories/:id/features
        // ? Booking routes (no auth required)
        .route("/directories/:slug/businesses/:business_id/available-slots", get(bookings::get_available_slots))
        .route("/directories/:slug/businesses/:business_id/book", post(bookings::create_booking))
        // ? Public booking page (no auth required, also outside the auth middleware)
        .route("/book/:slug/:business_id", get(booking_page::booking_page))
        // ? Stage 5: Service Booking system (visitor booking flow)
        .route("/bookings", post(bookings::create_service_booking).get(bookings::list_visitor_bookings))
        .route("/bookings/:id", get(bookings::get_booking))
        .route("/bookings/:id/status", post(bookings::update_booking_status))
        .route("/bookings/:id/cancel", post(bookings::cancel_booking))
        .route("/business/:business_id/bookings", get(bookings::list_business_bookings))
        // ? Stage 5: Service Catalog (business services/products)
        .route("/services", get(service_catalog::list_services).post(service_catalog::create_service))
        .route("/services/:id", get(service_catalog::get_service).put(service_catalog::update_service).delete(service_catalog::delete_service))
        .route("/businesses/:business_id/services", get(service_catalog::list_services_for_business))
        // ? My Bookings server-rendered page
        .route("/my-bookings", get(public::my_bookings_page))
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
        // ??? City Requests admin endpoints
        .route("/admin/directories/:id/city-requests", get(visitors::admin_get_city_requests))
        .route("/admin/directories/:id/city-requests/:request_id/mark-added", post(visitors::admin_mark_city_added))
        // ? Business messaging - owner routes (auth required)
        .route("/messages/:business_id", get(messaging::list_messages))
        .route("/messages/:business_id/unread", get(messaging::unread_count))
        .route("/messages/:id/read", patch(messaging::mark_read))
        .layer(middleware::from_fn_with_state(
            s.clone(),
            auth_guard,
        ));

    // ??? Serve SPA frontend at root
    // Resolve the frontend directory at runtime so it works both in Docker
    // (bind-mounted at /opt/swift/multidirectory-rust/frontend) and on baremetal
    // (checked out at /opt/swift/apps/multi-directory/frontend).
    let frontend_path = [
        "/opt/swift/multidirectory-rust/frontend",
        "/opt/swift/apps/multi-directory/frontend",
        "./frontend",
    ]
    .iter()
    .map(|p| std::path::Path::new(p))
    .find(|p| p.join("index.html").exists())
    .map(|p| p.to_string_lossy().to_string())
    .unwrap_or_else(|| "/opt/swift/multidirectory-rust/frontend".to_string());

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
        // Trap door public pages
        .route("/p/:slug", get(trap_doors::serve_trap_door_page))
        // Dynamic OG images also available at root level (mirrors /api/v1/public/og/...)
        .route("/public/og/:page_type/:page_id", get(dynamic_og::dynamic_og_image))
        // ??? ZaarHub SSR city landing pages (public, SEO-optimized) — must be before fallback
        .route("/zaarhub/:slug/:id", get(zaarhub_ssr::render_listing_page))
        .route("/zaarhub/:slug", get(zaarhub_ssr::render_city_page))
        .route("/zaarhub", get(zaarhub_ssr::render_cities_index))
        // ??? ZaarHub legal pages (public SSR)
        .route("/legal/:slug", get(zaarhub_ssr::render_legal_page))
        // ??? B2B Moat SSR pages (public: RFQ marketplace, co-op hub, lead exchange)
        .route("/rfq-marketplace", get(b2b_ssr::render_rfq_marketplace))
        .route("/rfq-marketplace/", get(b2b_ssr::render_rfq_marketplace))
        .route("/coop-hub", get(b2b_ssr::render_coop_hub))
        .route("/coop-hub/", get(b2b_ssr::render_coop_hub))
        .route("/lead-exchange", get(b2b_ssr::render_lead_exchange))
        .route("/lead-exchange/", get(b2b_ssr::render_lead_exchange))
        // ??? ZaarHub SEO (sitemap + robots)
        .route("/zaarhub-sitemap.xml", get(zaarhub_seo::sitemap_xml))
        .route("/zaarhub-robots.txt", get(zaarhub_seo::robots_txt))
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
                            || host == "localhost" || host == "directory.swiftsoftware.net"
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
                    if path == "/admin/login" || path == "/login" || path == "/login.html" || path == "/admin/" || path == "/admin" {
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

                    // Serve admin panel dashboard
                    if path == "/admin-panel" || path == "/admin-panel.html" || path == "/admin-panel/" {
                        let admin_path = std::path::Path::new(&frontend).join("admin-panel.html");
                        if admin_path.exists() {
                            match tokio::fs::read(&admin_path).await {
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

                    // Serve blog features admin panel
                    if path == "/blog-features" || path == "/blog-features/" {
                        let blog_feat_path = std::path::Path::new(&frontend).join("blog-features-admin.html");
                        if blog_feat_path.exists() {
                            match tokio::fs::read(&blog_feat_path).await {
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

                    // Serve content research admin panel
                    if path == "/research" || path == "/research/" {
                        let research_path = std::path::Path::new(&frontend).join("content-research.html");
                        if research_path.exists() {
                            match tokio::fs::read(&research_path).await {
                                Ok(html) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                                            .body(axum::body::Body::from(html))
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

                    // ??? Serve public legal terms page
                    if path == "/legal-terms" || path == "/legal-terms/" {
                        let legal_path = std::path::Path::new(&frontend).join("legal-terms.html");
                        if legal_path.exists() {
                            match tokio::fs::read(&legal_path).await {
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

                    // ??? Serve supplier portal (B2B: distributors, wholesalers, farms, associations)
                    if path == "/supplier" || path == "/supplier/" || path.starts_with("/supplier/dashboard") || path == "/distributor" || path == "/distributor/" || path.starts_with("/distributor/dashboard") {
                        let portal_path = std::path::Path::new(&frontend).join("supplier-portal.html");
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

                    // ??? Serve browsable Supplier Directory (auth-gated B2B directory for business owners)
                    if path == "/supplier-directory" || path == "/supplier-directory/" || path == "/suppliers" || path == "/suppliers/" {
                        let dir_path = std::path::Path::new(&frontend).join("supplier-directory.html");
                        if dir_path.exists() {
                            match tokio::fs::read(&dir_path).await {
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

                    // ??? ZaarHub: Business detail page (/biz/:id)
                    if path.starts_with("/biz/") {
                        let detail_path = std::path::Path::new(&frontend).join("business-detail.html");
                        if detail_path.exists() {
                            match tokio::fs::read(&detail_path).await {
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

                    // ??? ZaarHub: Business owner landing page
                    if path == "/grow" || path == "/grow/" {
                        let grow_path = std::path::Path::new(&frontend).join("grow.html");
                        if grow_path.exists() {
                            match tokio::fs::read(&grow_path).await {
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

                    // ??? ZaarHub: Claim listing page
                    if path == "/claim" || path == "/claim/" || path.starts_with("/claim?") {
                        let claim_path = std::path::Path::new(&frontend).join("claim.html");
                        if claim_path.exists() {
                            match tokio::fs::read(&claim_path).await {
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

                    // ??? ZaarHub: Loyalty Scanner (business QR scan + purchase verification)
                    if path == "/scanner" || path == "/scanner/" || path == "/scan" || path == "/scan/" {
                        let scanner_path = std::path::Path::new(&frontend).join("scanner.html");
                        if scanner_path.exists() {
                            match tokio::fs::read(&scanner_path).await {
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

                    // ??? Scanner PWA assets — manifest and service worker
                    if path == "/scanner-manifest.json" {
                        let mf_path = std::path::Path::new(&frontend).join("scanner-manifest.json");
                        if mf_path.exists() {
                            match tokio::fs::read(&mf_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "application/manifest+json")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    if path == "/sw-scanner.js" {
                        let sw_path = std::path::Path::new(&frontend).join("sw-scanner.js");
                        if sw_path.exists() {
                            match tokio::fs::read(&sw_path).await {
                                Ok(content) => {
                                    return Ok::<_, std::convert::Infallible>(
                                        axum::response::Response::builder()
                                            .status(axum::http::StatusCode::OK)
                                            .header(axum::http::header::CONTENT_TYPE, "application/javascript; charset=utf-8")
                                            .header("Service-Worker-Allowed", "/")
                                            .body(axum::body::Body::from(content))
                                            .unwrap()
                                    );
                                }
                                Err(_) => {}
                            }
                        }
                    }

                    // ??? ZaarHub: User saved/reviews/perks dashboard
                    if path == "/user/saved" || path == "/user/saved/" {
                        let user_path = std::path::Path::new(&frontend).join("user-saved.html");
                        if user_path.exists() {
                            match tokio::fs::read(&user_path).await {
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

                    // &#10052;&#65039; ZaarHub public community frontend
                    if path == "/zaarhub" || path == "/zaarhub/" || path.starts_with("/zaarhub/") || path == "/z" || path == "/z/" || path.starts_with("/z/") {
                        let zh_path = std::path::Path::new(&frontend).join("zaarhub.html");
                        if zh_path.exists() {
                            match tokio::fs::read(&zh_path).await {
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

                    // Redirect /saved-places to /api/v1/saved-places (server-rendered page)
                    if path == "/saved-places" {
                        return Ok::<_, std::convert::Infallible>(
                            axum::response::Response::builder()
                                .status(axum::http::StatusCode::FOUND)
                                .header("Location", "/api/v1/saved-places")
                                .body(axum::body::Body::empty())
                                .unwrap()
                        );
                    }

                    // Redirect /feed to /api/v1/feed-page (server-rendered neighborhood feed)
                    if path == "/feed" || path == "/feed/" {
                        return Ok::<_, std::convert::Infallible>(
                            axum::response::Response::builder()
                                .status(axum::http::StatusCode::FOUND)
                                .header("Location", "/api/v1/feed-page")
                                .body(axum::body::Body::empty())
                                .unwrap()
                        );
                    }

                    // Redirect /events to /api/v1/events-page (server-rendered events calendar)
                    if path == "/events" || path.starts_with("/events?") {
                        let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
                        return Ok::<_, std::convert::Infallible>(
                            axum::response::Response::builder()
                                .status(axum::http::StatusCode::FOUND)
                                .header("Location", format!("/api/v1/events-page{}", query))
                                .body(axum::body::Body::empty())
                                .unwrap()
                        );
                    }

                    // SPA fallback: serve full index.html for all unmatched routes
                    {
                        // White-label: if the path targets a directory (/d/{slug}),
                        // inject that directory's branding into the served HTML.
                        let mut html = index_clone2.as_ref().to_string();
                        let slug = crate::branding_injector::slug_from_dir_path(&path);
                        let branding =
                            if let Some(slug) = slug {
                                crate::branding_injector::fetch_branding_by_slug(
                                    &_pool_for_host,
                                    &slug,
                                )
                                .await
                            } else {
                                None
                            };
                        html = crate::branding_injector::inject_branding(&html, branding.as_ref());
                        return Ok(axum::response::Response::builder()
                            .status(axum::http::StatusCode::OK)
                            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                            .body(axum::body::Body::from(html))
                            .unwrap());
                    }
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
        || path.starts_with("/d/")
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
        // Public business message sending (guests can send messages)
        || (path.starts_with("/messages/") && req.method() == "POST")
        // Public data pipeline ingest (external sources push here)
        || path == "/pipeline/ingest"
        // Public community posts (GET only, POST/PUT/DELETE need auth)
        || (path == "/community/posts" && req.method() == "GET")
        || (path.starts_with("/community/posts/") && req.method() == "GET")
        // Public B2B marketplace (read-only, POST/PUT/DELETE need auth)
        || (path == "/b2b/products" && req.method() == "GET")
        || (path.starts_with("/b2b/products/") && req.method() == "GET")
        || path == "/b2b/suppliers"
        // Public B2B marketplace & discovery (read-only)
        || (path == "/b2b/marketplace" && req.method() == "GET")
        || (path.starts_with("/b2b/suppliers/") && path.ends_with("/detail") && req.method() == "GET")
        || (path == "/b2b/discover" && req.method() == "GET")
        // Public RFQ marketplace (read-only)
        || (path == "/b2b/rfqs/stats" && req.method() == "GET")
        || (path == "/b2b/rfqs" && req.method() == "GET")
        || (path.starts_with("/b2b/rfqs/") && req.method() == "GET")
        // Public lead sharing (read-only)
        || (path == "/b2b/leads/available" && req.method() == "GET")
        // Public co-op groups + deals (read-only)
        || (path == "/b2b/co-op/groups" && req.method() == "GET")
        || (path.starts_with("/b2b/co-op/groups/") && req.method() == "GET" && !path.ends_with("/join") && !path.ends_with("/deals"))
        || (path == "/b2b/co-op/deals/active" && req.method() == "GET")
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
        // Public deals browsing (visitors browse and claim deals without auth)
        || (path == "/deals" && req.method() == "GET")
        || (path == "/deals/featured" && req.method() == "GET")
        || (path.starts_with("/deals/") && path.ends_with("/claim") && req.method() == "POST")
        // Public deal detail pages (GET /deals/:uuid)
        || (path.starts_with("/deals/") && req.method() == "GET" && path.matches('/').count() == 2)
        // Public deal detail page data (GET /deals/:uuid/page)
        || (path.starts_with("/deals/") && path.ends_with("/page") && req.method() == "GET")
        // Public featured deals
        || (path.ends_with("/features") && req.method() == "GET")
        // Public business claim form
        || (path.starts_with("/businesses/") && path.ends_with("/claim") && req.method() == "POST")
        // Visitor favorites/bookmarks (handlers handle their own auth extraction)
        || (path == "/visitor/favorites" && req.method() == "GET")
        || (path.starts_with("/visitor/favorites/") && req.method() == "POST")
        || (path.starts_with("/visitor/favorites/check/") && req.method() == "GET")
        // Public bookmark endpoints
        || (path == "/bookmarks" && req.method() == "GET")
        || (path == "/bookmarks/toggle" && req.method() == "POST")
        || (path.starts_with("/bookmarks/count/") && req.method() == "GET")
        // Server-rendered saved places page (handlers handle auth extraction)
        || path == "/saved-places"
        // Cron endpoints (triggered by cron daemon with optional API key)
        || (path == "/cron/contact-intelligence" && req.method() == "POST")
        || (path == "/cron/content-queue-worker" && req.method() == "POST")
        // Public business image upload
        || (path.starts_with("/businesses/") && path.ends_with("/images") && req.method() == "POST")
        // Public city requests
        || path == "/city-requests"
        // Public poll endpoints (handlers handle their own auth extraction)
        || (path == "/polls" && req.method() == "GET")
        || (path.starts_with("/polls/") && req.method() == "GET")
        || (path.starts_with("/polls/") && path.ends_with("/vote") && req.method() == "POST")
        || (path.starts_with("/polls/") && path.ends_with("/close") && req.method() == "POST")
        // Public community events (list and get are public; RSVP/cancel/edit handle auth internally)
        || (path == "/events" && req.method() == "GET")
        || (path.starts_with("/events/") && req.method() == "GET" && !path.contains("/attendees"))
        || (path.starts_with("/events/") && path.ends_with("/rsvp") && req.method() == "POST")
        || (path.starts_with("/events/") && path.ends_with("/cancel") && req.method() == "POST")
        || (path.starts_with("/events/") && path.ends_with("/edit") && req.method() == "POST")
        // Public events-page (server-rendered, handles auth internally)
        || (path.starts_with("/events-page") && req.method() == "GET")
        // Feed routes (handlers handle their own auth extraction)
        || path == "/feed"
        || (path.starts_with("/feed-page") && req.method() == "GET")
        // Public booking endpoints
        || (path.contains("/available-slots") && req.method() == "GET")
        || (path.contains("/book") && req.method() == "POST" && !path.contains("blog"))
        // Public booking page (GET)
        || (path.starts_with("/book/") && req.method() == "GET")
        // Public bookmark count (no auth)
        || (path.starts_with("/bookmarks/count/") && req.method() == "GET")
        // Public Google Places search (admin populate tool — read-only lookups)
        || path == "/places/autocomplete"
        || path == "/places/details"
        || path == "/api/v1/places/autocomplete"
        || path == "/api/v1/places/details"
        // ZaarHub community frontend API (public)
        || path.starts_with("/zaarhub/")
        || path.starts_with("/zaarhub-sitemap.xml")
        || path.starts_with("/api/v1/zaarhub/")
        || path.starts_with("/legal/")
        // Public B2B SSR pages (RFQ marketplace, co-op hub, lead exchange)
        || path == "/rfq-marketplace" || path == "/rfq-marketplace/"
        || path == "/coop-hub" || path == "/coop-hub/"
        || path == "/lead-exchange" || path == "/lead-exchange/"
        // Public ad rendering (no auth)
        || path.starts_with("/ads/")
        // Public spotlight & notifications (Phase 4)
        || path.starts_with("/spotlight/")
        || path.starts_with("/notifications/")
        // Stage 5: Server-rendered my-bookings page (handles auth internally)
        || path == "/my-bookings"
        // Stage 5: SSO (handlers authenticate internally)
        || path == "/auth/switch-role"
        || path == "/auth/linked-accounts";
    
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

