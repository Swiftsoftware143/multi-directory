#![allow(clippy::all)]
#![allow(unused)]
#![recursion_limit = "256"]
mod coreswift;
mod email;
mod reminders;

mod auth;
mod beacon_middleware;
mod branding_injector;
mod config;
mod db;
mod error;
mod handlers;
mod models;
mod providers;
mod routes;
mod security;
mod state;
mod template_engine;
pub mod tracking_script;
mod utils;

use axum::Router;
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

    // Connect to IncentiveSwift database as well
    let is_db_url = std::env::var("IS_DATABASE_URL")
        .expect("IS_DATABASE_URL must be set (IncentiveSwift DB for loyalty integration)");
    let is_db = db::connect(
        &is_db_url,
        config.db_min_connections,
        config.db_max_connections,
    )
    .await;

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

    let state = AppState::new(pool, config.clone(), is_db);

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

    axum::serve(listener, app)
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
