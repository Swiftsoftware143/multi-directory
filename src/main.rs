mod email;

mod config;
mod db;
mod error;
mod state;
mod models;
mod handlers;
mod auth;
mod routes;
mod template_engine;

use axum::Router;
use std::time::Duration;
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

pub use state::AppState;
pub use error::AppError;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(true)
        .with_thread_ids(true)
        .init();

    let config = config::AppConfig::from_env();
    let pool = db::connect(&config.database_url, config.db_min_connections, config.db_max_connections).await;

    // Run migrations
    tracing::info!("Running database migrations...");
    db::run_migrations(&pool).await;

    let state = AppState::new(pool, config.clone());

    let app = routes::create_router(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Starting Multi-Directory API server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.expect("Failed to bind address");

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
