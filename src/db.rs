//! Database connection and migration runner.
//!
//! Hardened 2026-09-20 after a production outage: the container user could not
//! traverse the bind-mounted `src/migrations` directory, `read_dir` failed and
//! the `.expect(...)` below crash-looped the app. Two principles now hold:
//!
//!   1. **A migration problem never takes the service down.** An unreadable or
//!      missing migrations directory logs a warning and startup continues.
//!   2. **A migration is only recorded as applied if every statement actually
//!      succeeded.** Previously a file whose statements all failed was still
//!      inserted into `_migrations` (087/088/089 were marked applied while
//!      creating no tables at all, so they would never re-run). Failures are now
//!      recorded with `had_errors = true`, which keeps the file *visible* and
//!      *retryable* on the next boot.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

pub async fn connect(database_url: &str, min_connections: u32, max_connections: u32) -> PgPool {
    let options: PgConnectOptions = database_url.parse().expect("Invalid DATABASE_URL format");

    PgPoolOptions::new()
        .min_connections(min_connections)
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .expect("Failed to connect to database")
}

/// Split a migration file into individually executable statements.
///
/// A naive `split(';')` tears function bodies apart: `CREATE FUNCTION ... AS $$
/// BEGIN ...; END; $$ LANGUAGE plpgsql;` contains semicolons that are *not*
/// statement terminators. This splitter tracks:
///   * single-quoted strings (with `''` escaping),
///   * double-quoted identifiers (with `""` escaping),
///   * dollar-quoted blocks (`$$…$$`, `$tag$…$tag$`),
///   * line comments (`-- …`) and block comments (`/* … */`, nestable),
///
/// and only treats `;` as a separator in normal state.
pub fn split_statements(sql: &str) -> Vec<String> {
    #[derive(PartialEq)]
    enum St {
        Normal,
        Single,
        Double,
        LineComment,
        BlockComment(usize),
        Dollar(String),
    }

    let chars: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut state = St::Normal;
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];

        match &state {
            St::Normal => {
                if c == '-' && chars.get(i + 1) == Some(&'-') {
                    state = St::LineComment;
                    current.push(c);
                } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                    state = St::BlockComment(1);
                    current.push(c);
                } else if c == '\'' {
                    state = St::Single;
                    current.push(c);
                } else if c == '"' {
                    state = St::Double;
                    current.push(c);
                } else if c == '$' {
                    // Dollar quote: $tag$ where tag is [A-Za-z0-9_]* (possibly empty).
                    let mut j = i + 1;
                    let mut tag = String::new();
                    while let Some(&nc) = chars.get(j) {
                        if nc == '$' {
                            break;
                        }
                        if nc.is_ascii_alphanumeric() || nc == '_' {
                            tag.push(nc);
                            j += 1;
                        } else {
                            tag.clear();
                            j = i; // not a valid dollar quote
                            break;
                        }
                    }
                    if chars.get(j) == Some(&'$') {
                        // Enter dollar-quoted block, consuming the opening delimiter.
                        for k in i..=j {
                            current.push(chars[k]);
                        }
                        state = St::Dollar(tag);
                        i = j + 1;
                        continue;
                    } else {
                        current.push(c);
                    }
                } else if c == ';' {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        statements.push(trimmed.to_string());
                    }
                    current.clear();
                } else {
                    current.push(c);
                }
            }
            St::Single => {
                current.push(c);
                if c == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        current.push('\'');
                        i += 1; // escaped quote '' stays inside the string
                    } else {
                        state = St::Normal;
                    }
                }
            }
            St::Double => {
                current.push(c);
                if c == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        current.push('"');
                        i += 1;
                    } else {
                        state = St::Normal;
                    }
                }
            }
            St::LineComment => {
                current.push(c);
                if c == '\n' {
                    state = St::Normal;
                }
            }
            St::BlockComment(depth) => {
                current.push(c);
                let depth = *depth;
                if c == '/' && chars.get(i + 1) == Some(&'*') {
                    current.push('*');
                    i += 1;
                    state = St::BlockComment(depth + 1);
                } else if c == '*' && chars.get(i + 1) == Some(&'/') {
                    current.push('/');
                    i += 1;
                    if depth <= 1 {
                        state = St::Normal;
                    } else {
                        state = St::BlockComment(depth - 1);
                    }
                }
            }
            St::Dollar(tag) => {
                current.push(c);
                if c == '$' {
                    // Closing delimiter is $tag$ — check the following chars.
                    let mut j = i + 1;
                    let mut ok = true;
                    for tc in tag.chars() {
                        if chars.get(j) == Some(&tc) {
                            j += 1;
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    if ok && chars.get(j) == Some(&'$') {
                        current.extend(tag.chars());
                        current.push('$');
                        i = j + 1;
                        state = St::Normal;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }

    statements
}

pub async fn run_migrations(pool: &PgPool) {
    let mut migration_dir = std::path::Path::new("./src/migrations");
    if !migration_dir.is_dir() {
        migration_dir = std::path::Path::new("./migrations");
    }
    if !migration_dir.is_dir() {
        tracing::warn!(
            "migrations: no migrations directory found (looked at ./src/migrations and ./migrations) — continuing startup without migrations"
        );
        return;
    }

    // NEVER panic here. An unreadable directory (permissions, a missing
    // bind-mount, a bad owner) must degrade to a warning, not an outage.
    let read = match std::fs::read_dir(migration_dir) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(
                "migrations: cannot read {} ({}) — continuing startup WITHOUT applying migrations",
                migration_dir.display(),
                e
            );
            return;
        }
    };

    let mut entries: Vec<std::path::PathBuf> = read
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "sql").unwrap_or(false))
        .collect();

    entries.sort();

    // Create migrations tracking table. A failure here is logged, not fatal.
    if let Err(e) = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS _migrations (
            id SERIAL PRIMARY KEY,
            filename VARCHAR(255) NOT NULL UNIQUE,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
    )
    .execute(pool)
    .await
    {
        tracing::warn!(
            "migrations: cannot ensure _migrations table ({}) — skipping migrations",
            e
        );
        return;
    }

    // Visibility for a migration whose statements failed: it stays in the ledger
    // but flagged, so it is retried on the next boot instead of being silently
    // assumed done.
    if let Err(e) = sqlx::query(
        "ALTER TABLE _migrations ADD COLUMN IF NOT EXISTS had_errors BOOLEAN NOT NULL DEFAULT false",
    )
    .execute(pool)
    .await
    {
        tracing::warn!("migrations: cannot add _migrations.had_errors column ({})", e);
    }

    let mut applied = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for path in &entries {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        // already applied (and not a previously-failed file) -> skip
        match sqlx::query_scalar::<_, bool>(
            "SELECT had_errors FROM _migrations WHERE filename = $1",
        )
        .bind(&filename)
        .fetch_optional(pool)
        .await
        {
            Ok(Some(false)) => {
                tracing::info!("Migration {} already applied, skipping", filename);
                skipped += 1;
                continue;
            }
            Ok(Some(true)) => {
                tracing::warn!("Migration {} previously failed — retrying", filename);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    "migrations: cannot read ledger for {} ({}) — skipping this file",
                    filename,
                    e
                );
                skipped += 1;
                continue;
            }
        }

        let sql = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("migrations: cannot read {} ({}) — skipping", filename, e);
                skipped += 1;
                continue;
            }
        };

        tracing::info!("Applying migration: {}", filename);

        let statements = split_statements(&sql);
        let mut errors: Vec<String> = Vec::new();

        for statement in &statements {
            if let Err(e) = sqlx::query(statement).execute(pool).await {
                tracing::warn!(
                    "Migration {} statement failed: {} | stmt: {}",
                    filename,
                    e,
                    statement.chars().take(160).collect::<String>()
                );
                errors.push(e.to_string());
            }
        }

        // Record with had_errors=true when anything failed: visible in the
        // ledger AND retried next boot. Only a fully-clean file is marked done.
        let had_errors = !errors.is_empty();
        let ledger = sqlx::query(
            "INSERT INTO _migrations (filename, had_errors) VALUES ($1, $2)
             ON CONFLICT (filename) DO UPDATE SET applied_at = NOW(), had_errors = EXCLUDED.had_errors",
        )
        .bind(&filename)
        .bind(had_errors)
        .execute(pool)
        .await;

        if let Err(e) = ledger {
            // Never swallow a ledger write error silently.
            tracing::warn!(
                "migrations: cannot record {} in _migrations ({}) — it will retry next boot",
                filename,
                e
            );
            failed += 1;
            continue;
        }

        if had_errors {
            failed += 1;
            tracing::warn!(
                "Migration {} FAILED ({} statement error(s)) — left unapplied/retryable",
                filename,
                errors.len()
            );
        } else {
            applied += 1;
        }
    }

    tracing::info!(
        "migrations: applied {}, skipped {}, failed {}",
        applied,
        skipped,
        failed
    );
}

#[cfg(test)]
mod tests {
    use super::split_statements;

    #[test]
    fn splits_plain_statements() {
        let sql = "CREATE TABLE a (id int);\nCREATE TABLE b (id int);\n";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("CREATE TABLE a"));
        assert!(stmts[1].contains("CREATE TABLE b"));
    }

    #[test]
    fn keeps_dollar_quoted_bodies_intact() {
        let sql = "CREATE TABLE t (id int);
CREATE FUNCTION f() RETURNS trigger AS $$
BEGIN
    IF NEW.x IS NULL THEN
        NEW.x := 1;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE INDEX IF NOT EXISTS i ON t(id);
";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 3, "statements: {:?}", stmts);
        assert!(stmts[1].starts_with("CREATE FUNCTION"));
        assert!(stmts[1].contains("$$ LANGUAGE plpgsql"));
        assert!(stmts[2].contains("CREATE INDEX"));
    }

    #[test]
    fn keeps_tagged_dollar_quotes_and_strings() {
        let sql = "INSERT INTO t (v) VALUES ('a;b''c');
CREATE FUNCTION g() RETURNS int AS $tag$
BEGIN
  RETURN 1;
END;
$tag$ LANGUAGE plpgsql;
";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("'a;b''c'"));
        assert!(stmts[1].contains("$tag$"));
    }

    #[test]
    fn ignores_semicolons_in_comments() {
        let sql = "-- not a terminator ;\nCREATE TABLE c (id int); /* also ; here */\nCREATE TABLE d (id int);";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
    }
}
