//! Application configuration loaded from environment variables.

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_access_expiry: i64,
    pub jwt_refresh_expiry: i64,
    pub db_min_connections: u32,
    pub db_max_connections: u32,
    pub template_dir: String,
    pub base_domain: String,
    pub admin_email: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let host = std::env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = std::env::var("APP_PORT")
            .unwrap_or_else(|_| "3001".to_string())
            .parse::<u16>()
            .expect("Invalid APP_PORT");

        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL environment variable is required");

        let jwt_secret = std::env::var("JWT_SECRET")
            .expect("JWT_SECRET environment variable is required");

        let jwt_access_expiry = std::env::var("JWT_ACCESS_TOKEN_EXPIRY")
            .unwrap_or_else(|_| "86400".to_string())
            .parse::<i64>()
            .expect("Invalid JWT_ACCESS_TOKEN_EXPIRY");

        let jwt_refresh_expiry = std::env::var("JWT_REFRESH_TOKEN_EXPIRY")
            .unwrap_or_else(|_| "2592000".to_string())
            .parse::<i64>()
            .expect("Invalid JWT_REFRESH_TOKEN_EXPIRY");

        let db_min_connections = std::env::var("DB_MIN_CONNECTIONS")
            .unwrap_or_else(|_| "2".to_string())
            .parse::<u32>()
            .expect("Invalid DB_MIN_CONNECTIONS");

        let db_max_connections = std::env::var("DB_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u32>()
            .expect("Invalid DB_MAX_CONNECTIONS");

        let template_dir = std::env::var("TEMPLATE_DIR")
            .unwrap_or_else(|_| "./templates".to_string());

        let base_domain = std::env::var("BASE_DOMAIN")
            .unwrap_or_else(|_| "directory.swiftsoftware.net".to_string());

        let admin_email = std::env::var("ADMIN_EMAIL")
            .unwrap_or_else(|_| "admin@example.com".to_string());

        Self {
            host,
            port,
            database_url,
            jwt_secret,
            jwt_access_expiry,
            jwt_refresh_expiry,
            db_min_connections,
            db_max_connections,
            template_dir,
            base_domain,
            admin_email,
        }
    }
}
