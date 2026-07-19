//! Shared application state.

use sqlx::PgPool;
use crate::config::AppConfig;
use crate::template_engine::TemplateEngine;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: AppConfig,
    pub template_engine: std::sync::Arc<std::sync::Mutex<TemplateEngine>>,
}

impl AppState {
    pub fn new(db: PgPool, config: AppConfig) -> Self {
        let mut engine = TemplateEngine::new();
        engine.load_all();
        Self {
            db,
            config,
            template_engine: std::sync::Arc::new(std::sync::Mutex::new(engine)),
        }
    }
}
