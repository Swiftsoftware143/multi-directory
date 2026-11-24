//! Shared application state.

use crate::config::AppConfig;
use crate::template_engine::TemplateEngine;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: AppConfig,
    pub template_engine: std::sync::Arc<std::sync::Mutex<TemplateEngine>>,
    pub mcp_door: std::sync::Arc<mcp_door::Door>,
}

impl AppState {
    pub fn new(db: PgPool, config: AppConfig) -> Self {
        let mut engine = TemplateEngine::new();
        engine.load_all();
        // The MCP door is built once at startup and fails closed: an invalid registry (a
        // half-annotated tool) refuses boot rather than serve a broken door (kanban t_6a165e41).
        let mcp_door = std::sync::Arc::new(
            crate::mcp::build_door(db.clone())
                .expect("MCP door registry is invalid — refusing to start"),
        );
        Self {
            db,
            config,
            template_engine: std::sync::Arc::new(std::sync::Mutex::new(engine)),
            mcp_door,
        }
    }
}
