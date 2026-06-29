//! Database connection and migration runner.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

pub async fn connect(database_url: &str, min_connections: u32, max_connections: u32) -> PgPool {
    let options: PgConnectOptions = database_url
        .parse()
        .expect("Invalid DATABASE_URL format");

    PgPoolOptions::new()
        .min_connections(min_connections)
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .expect("Failed to connect to database")
}

pub async fn run_migrations(pool: &PgPool) {
    let mut migration_dir = std::path::Path::new("./src/migrations");
    if !migration_dir.exists() {
        migration_dir = std::path::Path::new("./migrations");
    }
    if !migration_dir.exists() {
        tracing::warn!("Migrations directory not found, skipping");
        return;
    }

    let mut entries: Vec<_> = std::fs::read_dir(migration_dir)
        .expect("Failed to read migrations directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "sql").unwrap_or(false))
        .collect();

    entries.sort_by_key(|e| e.file_name());

    // Create migrations tracking table
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS _migrations (
            id SERIAL PRIMARY KEY,
            filename VARCHAR(255) NOT NULL UNIQUE,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
    )
    .execute(pool)
    .await
    .expect("Failed to create migrations tracking table");

    for entry in &entries {
        let filename = entry.file_name().to_string_lossy().to_string();

        let already_applied = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM _migrations WHERE filename = \x241",
        )
        .bind(&filename)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

        if already_applied > 0 {
            tracing::info!("Migration {} already applied, skipping", filename);
            continue;
        }

        let sql = std::fs::read_to_string(entry.path())
            .expect(&format!("Failed to read migration file: {}", filename));

        tracing::info!("Applying migration: {}", filename);

        for statement in sql.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                if let Err(e) = sqlx::query(trimmed).execute(pool).await {
                    tracing::warn!(
                        "Migration {} statement warning (may be non-fatal): {}",
                        filename, e
                    );
                }
            }
        }

        sqlx::query("INSERT INTO _migrations (filename) VALUES (\x241)")
            .bind(&filename)
            .execute(pool)
            .await
            .expect("Failed to record migration");
    }

    tracing::info!("All migrations applied successfully");
}
