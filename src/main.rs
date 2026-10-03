#![allow(clippy::all)]
#![allow(unused)]
#![recursion_limit = "256"]
mod coreswift;
mod email;
mod entitlements;
mod reminders;

mod auth;
mod beacon_middleware;
mod body_deadline;
mod brand_theme;
mod branding_injector;
mod business_types;
mod config;
mod db;
mod error;
mod handlers;
mod merge_fields;
mod models;
mod probe_harness;
mod providers;
mod routes;
mod security;
mod state;
mod system_tenant;
mod template_engine;
pub mod tracking_script;
mod utils;

use axum::Router;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

pub use error::AppError;
pub use state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(true)
        .with_thread_ids(true)
        .init();

    let config = config::AppConfig::from_env();
    let pool = db::connect(
        &config.database_url,
        config.db_min_connections,
        config.db_max_connections,
    )
    .await;

    // There is deliberately NO second database connection here. Multi-Directory used to
    // require IS_DATABASE_URL and open a live PgPool into the IncentiveSwift database for
    // the retired IQS funnel proxy / incentive-loyalty lookup. That coupling is gone: loyalty
    // is native Multi-Directory code (ZaarCash, tables loyalty_*) and CoreSwift CRM is the
    // only external integration, reached over HTTP. A buyer's install therefore boots with
    // DATABASE_URL alone.

    // Run migrations
    tracing::info!("Running database migrations...");
    db::run_migrations(&pool).await;

    // BYOK credentials (provider_keys.api_key) are encrypted at rest under an env-only master
    // key. Say the posture out loud at boot: a missing key means provider-key writes fail
    // closed (never a plaintext write), and reads report the provider as unconfigured.
    if security::provider_key_crypto::is_configured() {
        tracing::info!("Provider key encryption: enabled (AES-256 at rest, enc:v1 format)");
    } else {
        tracing::error!(
            "Provider key encryption: DISABLED — PROVIDER_KEY_ENC_SECRET missing or shorter than \
             32 chars. Storing a provider key will fail rather than store it in the clear."
        );
    }

    let state = AppState::new(pool, config.clone());

    // Say the inbound body bound out loud at boot (kanban t_52b9f3c7): a client that sends a
    // request head with a declared body and then stops is answered 408 at this bound instead of
    // parking a task, a connection and a partially-read body buffer for ever.
    tracing::info!(
        "Request body-read deadline: {}s on every route that reads a body, 408 above that \
         (BODY_READ_DEADLINE_SECS overrides, clamped 5..=300)",
        config.body_read_deadline_secs
    );

    // Subfolder SEO: report any directory whose slug shadows a top-level route
    // (that directory can then not be reached at `/<slug>`). Silent when clean;
    // also queryable at GET /api/v1/seo/subfolder-clashes.
    {
        let clashes = handlers::subfolder::find_clashes(&state.db).await;
        for (slug, against) in &clashes {
            tracing::warn!(
                "SUBFOLDER CLASH: directory slug '{}' is shadowed by the top-level '{}' route — \
                 /{} will not serve the directory home (use /d/{})",
                slug,
                against,
                slug,
                slug
            );
        }
        if !clashes.is_empty() {
            tracing::warn!("{} subfolder slug clash(es) detected", clashes.len());
        }
    }

    // Start background reminder cron
    reminders::start_reminder_cron(state.db.clone());

    // Start the enrichment cycle scheduler (T4) — the rotating re-enrichment that
    // was previously only a comment. It checks every 5 minutes for a due
    // enrichment_settings row and runs it; disabled settings are never touched.
    handlers::enrichment::start_enrichment_scheduler(state.db.clone());

    // Start the settlement scheduler (Round 14 / T1) — the monthly settlement and the
    // configured point-expiry policy driven by each network's own cycle_day, instead
    // of a button somebody has to remember to press. It calls the same
    // `settlement::execute_settlement` the manual endpoint does, and idempotency is
    // enforced by the database, so a scheduled run and a manual run cannot double-bill.
    handlers::settlement::start_settlement_scheduler(state.db.clone());

    let app = routes::create_router(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Starting Multi-Directory API server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    // kanban t_d5e5af7e: serve with ConnectInfo so `POST /api/v1/visitors/track` can record the
    // real request metadata instead of the placeholder "auto". The app binds 127.0.0.1 behind
    // nginx, so the handler prefers X-Forwarded-For / X-Real-IP and falls back to this peer addr.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("Server error");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Ctrl+C received, starting graceful shutdown");
        }
        _ = terminate => {
            tracing::info!("SIGTERM received, starting graceful shutdown");
        }
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    tracing::info!("Server shutdown complete");
}
