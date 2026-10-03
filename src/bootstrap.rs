//! First-run bootstrap for a self-hosted / handover install — kanban card B85.
//!
//! A fresh Multi-Directory database is created by `000_baseline_live_schema.sql` with **no
//! login row**: `users` is empty, so a buyer who takes the code onto their own server has no
//! way into the admin console. This module is that door. Two plain CLI subcommands, handled
//! before the HTTP server is built, so they run on a box that has NOT set JWT_SECRET or
//! PROVIDER_KEY_ENC_SECRET yet (only DATABASE_URL is required):
//!
//! ```text
//! multidirectory migrate                          # apply src/migrations (same path boot uses)
//! multidirectory create-admin --email you@x.com   # first super_admin; prints a generated
//!                                                 # password exactly once
//! ```
//!
//! Deliberate properties:
//! * never resets an existing account's password — re-running needs `--force`;
//! * the tenant is resolved BY SLUG (`swiftsoftware`, seeded by migration 001), never a
//!   hardcoded UUID (gate 5a);
//! * no `.unwrap()`/`.expect()` on any runtime value — every failure is a printed message and
//!   a non-zero exit.

use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// Exit codes: 0 ok, 1 runtime failure, 2 bad usage.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAIL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

/// Tenant the platform's own rows hang off (seeded by migration 001). Resolved by slug.
const DEFAULT_TENANT_SLUG: &str = "swiftsoftware";

struct Args {
    email: Option<String>,
    password: Option<String>,
    name: Option<String>,
    role: String,
    tenant_slug: String,
    database_url: Option<String>,
    force: bool,
    dry_run: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            email: None,
            password: None,
            name: None,
            role: "super_admin".to_string(),
            tenant_slug: DEFAULT_TENANT_SLUG.to_string(),
            database_url: None,
            force: false,
            dry_run: false,
        }
    }
}

/// The value following an option: `args[i]` is the option, so the value is at `i + 1`.
fn value_after(args: &[String], i: usize, what: &str) -> Result<String, String> {
    args.get(i + 1)
        .cloned()
        .ok_or_else(|| format!("{what} needs a value"))
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--email" | "-e" => {
                out.email = Some(value_after(args, i, "--email")?);
                i += 2;
            }
            "--password" | "-p" => {
                out.password = Some(value_after(args, i, "--password")?);
                i += 2;
            }
            "--name" => {
                out.name = Some(value_after(args, i, "--name")?);
                i += 2;
            }
            "--role" => {
                out.role = value_after(args, i, "--role")?;
                i += 2;
            }
            "--tenant" | "--tenant-slug" => {
                out.tenant_slug = value_after(args, i, "--tenant")?;
                i += 2;
            }
            "--database-url" => {
                out.database_url = Some(value_after(args, i, "--database-url")?);
                i += 2;
            }
            "--force" | "-f" => {
                out.force = true;
                i += 1;
            }
            "--dry-run" => {
                out.dry_run = true;
                i += 1;
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(out)
}

pub fn usage() {
    println!(
        "multidirectory — bootstrap commands (kanban B85)\n\
         \n\
         USAGE\n\
         \x20 multidirectory migrate [--database-url URL] [--dry-run]\n\
         \x20 multidirectory create-admin --email EMAIL [options]\n\
         \n\
         create-admin options\n\
         \x20 --email EMAIL          login address (required, must look like name@example.com)\n\
         \x20 --password PASS        use this password instead of generating one\n\
         \x20 --name NAME            display name (default: derived from the address)\n\
         \x20 --role ROLE            default: super_admin\n\
         \x20 --tenant SLUG          tenant to attach the account to (default: {DEFAULT_TENANT_SLUG})\n\
         \x20 --database-url URL     overrides $DATABASE_URL\n\
         \x20 --force                allow re-pointing an existing account's password\n\
         \x20 --dry-run              print what would happen, change nothing\n\
         \n\
         DATABASE_URL is read from the environment or a .env beside the binary."
    );
}

/// Resolve the database URL: --database-url, else $DATABASE_URL (after loading `.env`).
fn database_url(cli: &Args) -> Result<String, String> {
    if let Some(u) = cli.database_url.as_ref() {
        return Ok(u.clone());
    }
    let _ = dotenvy::dotenv();
    std::env::var("DATABASE_URL").map_err(|_| {
        "no database: pass --database-url or set DATABASE_URL (or put it in ./.env)".to_string()
    })
}

async fn connect(url: &str) -> Result<sqlx::PgPool, String> {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .map_err(|e| format!("cannot connect to the database: {e}"))
}

fn generate_password(len: usize) -> String {
    use rand::{distributions::Alphanumeric, Rng};
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// `multidirectory migrate` — apply the migrations the app applies at boot, then report the
/// ledger outcome (a file whose statements failed is retried on the next boot, so say so).
pub async fn run_migrate(args: &[String]) -> i32 {
    let cli = match parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            usage();
            return EXIT_USAGE;
        }
    };

    let url = match database_url(&cli) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_FAIL;
        }
    };

    if cli.dry_run {
        println!("dry-run: would apply ./src/migrations to the configured database");
        return EXIT_OK;
    }

    let pool = match connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_FAIL;
        }
    };

    println!("applying ./src/migrations ...");
    crate::db::run_migrations(&pool).await;

    // The ledger is the truth (see the migrations pitfall): a file recorded had_errors=true is
    // retried next boot and the schema may be missing it, so exit non-zero and say which.
    let bad: Vec<String> = match sqlx::query_scalar::<_, String>(
        "SELECT filename FROM _migrations WHERE had_errors ORDER BY filename",
    )
    .fetch_all(&pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: could not read the _migrations ledger: {e}");
            return EXIT_FAIL;
        }
    };
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _migrations")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    if bad.is_empty() {
        println!("OK: migrations applied, ledger holds {applied} file(s), 0 with errors");
        EXIT_OK
    } else {
        eprintln!(
            "FAIL: {} migration file(s) failed and will be retried on next boot: {}",
            bad.len(),
            bad.join(", ")
        );
        EXIT_FAIL
    }
}

/// `multidirectory create-admin` — create the first (or a replacement) privileged account.
pub async fn run_create_admin(args: &[String]) -> i32 {
    let cli = match parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            usage();
            return EXIT_USAGE;
        }
    };

    let email = match cli.email.as_deref() {
        Some(e) => e.to_string(),
        None => {
            eprintln!("error: --email is required");
            usage();
            return EXIT_USAGE;
        }
    };
    // Same normalisation every writer uses (t_01f183b1) so this row logs in.
    let email = match crate::security::email_addr::normalize(&email) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_USAGE;
        }
    };

    let url = match database_url(&cli) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_FAIL;
        }
    };

    let generated = cli.password.is_none();
    let password = cli
        .password
        .clone()
        .unwrap_or_else(|| generate_password(24));
    let name = cli
        .name
        .clone()
        .unwrap_or_else(|| email.split('@').next().unwrap_or("Admin").to_string());

    if cli.dry_run {
        println!(
            "dry-run: would create/replace role '{}' for {email} on tenant '{}'",
            cli.role, cli.tenant_slug
        );
        return EXIT_OK;
    }

    let pool = match connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_FAIL;
        }
    };

    // Tenant BY SLUG — never a baked UUID.
    let tenant_id: Uuid = match sqlx::query_scalar("SELECT id FROM tenants WHERE slug = $1")
        .bind(&cli.tenant_slug)
        .fetch_optional(&pool)
        .await
    {
        Ok(Some(id)) => id,
        Ok(None) => {
            eprintln!(
                "error: tenant '{}' does not exist — run `multidirectory migrate` first",
                cli.tenant_slug
            );
            return EXIT_FAIL;
        }
        Err(e) => {
            eprintln!("error: tenant lookup failed: {e}");
            return EXIT_FAIL;
        }
    };

    let existing: Option<(Uuid, String)> =
        match sqlx::query_as("SELECT id, role FROM users WHERE lower(email) = $1 LIMIT 1")
            .bind(&email)
            .fetch_optional(&pool)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: user lookup failed: {e}");
                return EXIT_FAIL;
            }
        };

    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let salt = SaltString::generate(&mut OsRng);
    let hash = match Argon2::default().hash_password(password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(e) => {
            eprintln!("error: could not hash the password: {e}");
            return EXIT_FAIL;
        }
    };

    if let Some((id, old_role)) = existing {
        if !cli.force {
            eprintln!(
                "error: {email} already exists (role '{old_role}'). Nothing was changed. \
                 Re-run with --force to reset its password, or use a different --email."
            );
            return EXIT_FAIL;
        }
        let res = sqlx::query(
            "UPDATE users SET password_hash = $1, role = $2, name = $3, is_active = true, \
             tenant_id = $4, updated_at = NOW() WHERE id = $5",
        )
        .bind(&hash)
        .bind(&cli.role)
        .bind(&name)
        .bind(tenant_id)
        .bind(id)
        .execute(&pool)
        .await;
        if let Err(e) = res {
            eprintln!("error: could not update the account: {e}");
            return EXIT_FAIL;
        }
        println!("updated {email} (id {id}) → role '{}'", cli.role);
    } else {
        let id = Uuid::new_v4();
        let res = sqlx::query(
            "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, true, NOW(), NOW())",
        )
        .bind(id)
        .bind(tenant_id)
        .bind(&email)
        .bind(&hash)
        .bind(&name)
        .bind(&cli.role)
        .execute(&pool)
        .await;
        if let Err(e) = res {
            eprintln!("error: could not create the account: {e}");
            return EXIT_FAIL;
        }
        println!("created {email} (id {id}) → role '{}'", cli.role);
    }

    println!("tenant:  {}", cli.tenant_slug);
    if generated {
        // Printed exactly once; only the Argon2 hash is stored.
        println!("password: {password}");
        println!("(store it now — it is not recoverable; reset it with --force)");
    } else {
        println!("password: (the one you passed)");
    }
    EXIT_OK
}
