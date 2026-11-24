//! Multi-Directory's MCP door: nine read-only tools, one admin/operator credential + per-directory
//! credentials (kanban t_6a165e41).
//!
//! Credential: `api_keys` (`md_…` keys). Two shapes, one resolver:
//!   * the OPERATOR key — `directory_id IS NULL` and the owning user is `super_admin` — sees every
//!     directory (Role::Admin) and may pass an optional `directory_id` filter to any list tool;
//!   * a DIRECTORY key — `directory_id` set, minted automatically the moment its directory is
//!     created — sees only that directory's listings/sponsors/ads (Role::Workspace, one tenant).
//!
//! The tenant→directory trap this door resolves: `api_keys.tenant_id` is a CUSTOMER tenant
//! (`tenants.id`), never a directory. The directory a key is bound to lives in the NEW
//! `api_keys.directory_id` column (migration 209), which is the single id every tool filters on.
//!
//! Read tools only. A foreign business id answers `not_found` (never `forbidden`) via
//! `Scope::require`; the operator's "see all" is an explicit `is_admin` branch, never an empty
//! scope widened to mean "everything". A directory key's query is filtered by its directory_id
//! INSIDE the statement (`WHERE ($1::uuid IS NULL OR directory_id = $1)`), fully parameterized.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::{DateTime, Utc};
use mcp_door::credential::constant_time_eq;
use mcp_door::{
    AuthError, Caller, CredentialResolver, Ctx, Door, Registry, Role, ToolError, ToolSpec,
};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::state::AppState;

/// The door's protocol identity.
pub const DOOR_NAME: &str = "multi-directory";
/// The audit file. Lives inside the container (WORKDIR `/app`, owned by the `multidirectory`
/// user) and is created `0600` on the first audited call.
pub const AUDIT_PATH: &str = "/app/mcp-audit.log";

const KEY_MARKER: &str = "md_";
const KEY_PREFIX_LEN: usize = 8;
/// Untrusted customer text (review bodies) is excerpt-capped.
const EXCERPT_CAP: usize = 200;

// ─────────────────────────────────────────────────────────────────────────────
// Resolver
// ─────────────────────────────────────────────────────────────────────────────

struct ApiKeyResolver {
    pool: PgPool,
}

/// The first `KEY_PREFIX_LEN` chars of the random part of an `md_<prefix>_<secret>` key.
fn issued_prefix(token: &str) -> Option<&str> {
    token
        .strip_prefix(KEY_MARKER)
        .and_then(|s| s.get(..KEY_PREFIX_LEN))
}

/// Compare a bearer against a stored SHA-256 key hash in constant time.
fn secret_matches(token: &str, stored: &str) -> bool {
    let digest = mcp_door::hash_token(token);
    constant_time_eq(digest.as_bytes(), stored.as_bytes())
}

#[async_trait]
impl CredentialResolver for ApiKeyResolver {
    async fn resolve(&self, bearer: &str) -> Result<Caller, AuthError> {
        let Some(prefix) = issued_prefix(bearer) else {
            return Err(AuthError::Invalid);
        };

        // key_hash, directory_id, key is_active, expires_at, owner role, owner is_active
        let rows: Vec<(
            String,
            Option<Uuid>,
            bool,
            Option<DateTime<Utc>>,
            String,
            bool,
        )> = sqlx::query_as(
            "SELECT k.key_hash, k.directory_id, COALESCE(k.is_active, true), k.expires_at, \
                        u.role, COALESCE(u.is_active, true) \
                   FROM api_keys k JOIN users u ON u.id = k.user_id \
                  WHERE k.key_prefix = $1",
        )
        .bind(prefix)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::Unavailable(e.to_string()))?;

        if rows.is_empty() {
            return Err(AuthError::Invalid);
        }

        let now = Utc::now();
        let mut has_live = false;
        let mut has_dead = false;
        for (key_hash, directory_id, key_active, expires_at, role, user_active) in rows {
            let expired = expires_at.is_some_and(|e| e <= now);
            if !key_active || expired || !user_active {
                has_dead = true;
                continue;
            }
            has_live = true;
            if !secret_matches(bearer, &key_hash) {
                continue;
            }

            match directory_id {
                // A directory-scoped key: the directory must still exist, else the key is revoked.
                Some(dir) => {
                    if !self.directory_live(dir).await? {
                        return Err(AuthError::Revoked);
                    }
                    return Ok(
                        Caller::new(format!("api_key:{prefix}"), Role::Workspace).with_tenant(dir)
                    );
                }
                // An unscoped key is the operator's alone: only a super_admin may see every
                // directory. A live key owned by anyone else is refused outright.
                None => {
                    if role != "super_admin" {
                        return Err(AuthError::Invalid);
                    }
                    return Ok(Caller::new(format!("operator:{prefix}"), Role::Admin));
                }
            }
        }

        // A live key with the wrong secret is a bad credential; revoked is the honest label only
        // when every row carrying this prefix is dead (inactive/expired/disabled owner).
        if has_live {
            Err(AuthError::Invalid)
        } else if has_dead {
            Err(AuthError::Revoked)
        } else {
            Err(AuthError::Invalid)
        }
    }
}

impl ApiKeyResolver {
    async fn directory_live(&self, dir: Uuid) -> Result<bool, AuthError> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM directories WHERE id = $1)")
                .bind(dir)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AuthError::Unavailable(e.to_string()))?;
        Ok(exists)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The directory filter a list tool applies. `None` = every directory (operator only; a
/// directory key always yields `Some`, so a list can never widen past its own directory).
fn list_directory(ctx: &Ctx, args: &Value) -> Result<Option<Uuid>, ToolError> {
    if ctx.is_admin() {
        match args.get("directory_id") {
            None => Ok(None),
            Some(v) => v
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(Some)
                .ok_or_else(|| ToolError::Invalid("`directory_id` is not a uuid".into())),
        }
    } else {
        ctx.scope
            .ids()
            .first()
            .copied()
            .map(Some)
            .ok_or_else(|| ToolError::Invalid("no directory bound to this credential".into()))
    }
}

/// The 404-not-403 gate for a single-row tool: an operator sees every directory; a directory key
/// may reach only its own, and a row in a foreign directory answers `not_found`.
fn require_directory(ctx: &Ctx, directory_id: Uuid) -> Result<(), ToolError> {
    if ctx.is_admin() {
        Ok(())
    } else {
        ctx.require_scope(directory_id)
    }
}

/// Untrusted customer text (review bodies) is excerpt-capped.
fn excerpt(text: Option<&str>) -> Option<String> {
    text.and_then(|t| {
        let t = t.trim();
        if t.is_empty() {
            None
        } else {
            let mut chars = t.chars();
            let head: String = chars.by_ref().take(EXCERPT_CAP).collect();
            if chars.next().is_some() {
                Some(format!("{head}…"))
            } else {
                Some(head)
            }
        }
    })
}

fn db_err(e: sqlx::Error) -> ToolError {
    ToolError::Unavailable(e.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tools
// ─────────────────────────────────────────────────────────────────────────────

fn registry(pool: PgPool) -> Registry {
    let mut reg = Registry::new();

    // 1 ── md_list_directories ────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_directories",
                "List directories (cities) this credential can see, newest first. An operator sees \
                 every directory and may pass an optional `directory_id`; a directory key sees only \
                 its own directory.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = sqlx::query(
                        "SELECT id, name, slug, city, state, status, template, network_id, url_type, \
                                custom_domain, created_at \
                           FROM directories \
                          WHERE ($1::uuid IS NULL OR id = $1) ORDER BY name LIMIT $2",
                    )
                    .bind(dir)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "name": r.get::<String, _>("name"),
                                "slug": r.get::<String, _>("slug"),
                                "city": r.get::<Option<String>, _>("city"),
                                "state": r.get::<Option<String>, _>("state"),
                                "status": r.get::<Option<String>, _>("status"),
                                "template": r.get::<Option<String>, _>("template"),
                                "network_id": r.get::<Option<Uuid>, _>("network_id"),
                                "url_type": r.get::<Option<String>, _>("url_type"),
                                "custom_domain": r.get::<Option<String>, _>("custom_domain"),
                                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 2 ── md_list_businesses ─────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_businesses",
                "List business listings in a directory, newest first. Returns name, city, primary \
                 category, status and verification flags — never a contact email. An operator may \
                 pass an optional `directory_id`; a directory key sees only its own listings.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = sqlx::query(
                        "SELECT b.id, b.name, b.slug, b.city, b.state, b.status, b.business_type, \
                                b.verified, b.featured, b.created_at, c.name AS category \
                           FROM businesses b \
                           LEFT JOIN business_categories bc \
                                  ON bc.business_id = b.id AND bc.is_primary = true \
                           LEFT JOIN directory_categories c ON c.id = bc.category_id \
                          WHERE ($1::uuid IS NULL OR b.directory_id = $1) \
                          ORDER BY b.created_at DESC LIMIT $2",
                    )
                    .bind(dir)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "name": r.get::<String, _>("name"),
                                "slug": r.get::<String, _>("slug"),
                                "city": r.get::<Option<String>, _>("city"),
                                "state": r.get::<Option<String>, _>("state"),
                                "status": r.get::<String, _>("status"),
                                "business_type": r.get::<Option<String>, _>("business_type"),
                                "category": r.get::<Option<String>, _>("category"),
                                "verified": r.get::<bool, _>("verified"),
                                "featured": r.get::<bool, _>("featured"),
                                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 3 ── md_get_business ────────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_get_business",
                "Fetch one business listing by id, with its walk score and verification status. A \
                 listing in another directory answers `not_found`.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let business_id = ctx.uuid_arg(&args, "business_id")?;

                    let row = sqlx::query(
                        "SELECT b.id, b.name, b.slug, b.description, b.address, b.city, b.state, \
                                b.zip, b.phone, b.email, b.website, b.status, b.business_type, \
                                b.verified, b.featured, b.rating, b.review_count, b.directory_id, \
                                b.created_at \
                           FROM businesses b WHERE b.id = $1",
                    )
                    .bind(business_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(db_err)?
                    .ok_or(ToolError::NotFound)?;

                    // Foreign (or missing) directory answers not_found, never forbidden.
                    let directory_id: Option<Uuid> = row.get("directory_id");
                    if let Some(dir) = directory_id {
                        require_directory(&ctx, dir)?;
                    } else if !ctx.is_admin() {
                        // A business with no directory cannot be reached by a directory key.
                        return Err(ToolError::NotFound);
                    }

                    let walk = sqlx::query(
                        "SELECT walk_score, bike_score, refreshed_at \
                           FROM business_walk_scores WHERE business_id = $1",
                    )
                    .bind(business_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(db_err)?;
                    let walk_score = walk.map(|r| {
                        json!({
                            "walk_score": r.get::<Option<i32>, _>("walk_score"),
                            "bike_score": r.get::<Option<i32>, _>("bike_score"),
                            "refreshed_at": r.get::<Option<DateTime<Utc>>, _>("refreshed_at"),
                        })
                    });

                    let verification = sqlx::query(
                        "SELECT method, status, verified_at \
                           FROM business_verifications WHERE business_id = $1 \
                          ORDER BY verified_at DESC LIMIT 1",
                    )
                    .bind(business_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(db_err)?;
                    let verification = verification.map(|r| {
                        json!({
                            "method": r.get::<String, _>("method"),
                            "status": r.get::<String, _>("status"),
                            "verified_at": r.get::<Option<DateTime<Utc>>, _>("verified_at"),
                        })
                    });

                    Ok(json!({
                        "id": row.get::<Uuid, _>("id"),
                        "name": row.get::<String, _>("name"),
                        "slug": row.get::<String, _>("slug"),
                        "description": excerpt(row.get::<Option<String>, _>("description").as_deref()),
                        "address": row.get::<Option<String>, _>("address"),
                        "city": row.get::<Option<String>, _>("city"),
                        "state": row.get::<Option<String>, _>("state"),
                        "zip": row.get::<Option<String>, _>("zip"),
                        "phone": row.get::<Option<String>, _>("phone"),
                        "email": row.get::<Option<String>, _>("email"),
                        "website": row.get::<Option<String>, _>("website"),
                        "status": row.get::<String, _>("status"),
                        "business_type": row.get::<Option<String>, _>("business_type"),
                        "verified": row.get::<bool, _>("verified"),
                        "featured": row.get::<bool, _>("featured"),
                        "rating": row.get::<Option<f64>, _>("rating"),
                        "review_count": row.get::<Option<i32>, _>("review_count"),
                        "directory_id": directory_id,
                        "created_at": row.get::<Option<DateTime<Utc>>, _>("created_at"),
                        "walk_score": walk_score,
                        "verification": verification,
                    }))
                })
            },
        );
    }

    // 4 ── md_list_categories ─────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_categories",
                "List the category taxonomy. An operator sees the whole global taxonomy (or, with \
                 `directory_id`, the categories its businesses use); a directory key sees only the \
                 categories its own businesses use.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = match dir {
                        Some(dir) => sqlx::query(
                            "SELECT DISTINCT c.id, c.name, c.slug, c.icon, c.group_name \
                               FROM directory_categories c \
                               JOIN business_categories bc ON bc.category_id = c.id \
                               JOIN businesses b ON b.id = bc.business_id \
                              WHERE b.directory_id = $1 ORDER BY c.name LIMIT $2",
                        )
                        .bind(dir)
                        .bind(limit)
                        .fetch_all(&pool)
                        .await
                        .map_err(db_err)?,
                        None => sqlx::query(
                            "SELECT id, name, slug, icon, group_name \
                               FROM directory_categories ORDER BY name LIMIT $1",
                        )
                        .bind(limit)
                        .fetch_all(&pool)
                        .await
                        .map_err(db_err)?,
                    };
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "name": r.get::<String, _>("name"),
                                "slug": r.get::<String, _>("slug"),
                                "icon": r.get::<Option<String>, _>("icon"),
                                "group_name": r.get::<Option<String>, _>("group_name"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 5 ── md_list_submissions ────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_submissions",
                "List business submissions, pending first. An operator may pass an optional \
                 `directory_id`; a directory key sees only its own submissions.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = sqlx::query(
                        "SELECT id, business_name, city, state, status, created_at \
                           FROM submissions \
                          WHERE ($1::uuid IS NULL OR directory_id = $1) \
                          ORDER BY (status = 'pending') DESC, created_at DESC LIMIT $2",
                    )
                    .bind(dir)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "business_name": r.get::<String, _>("business_name"),
                                "city": r.get::<Option<String>, _>("city"),
                                "state": r.get::<Option<String>, _>("state"),
                                "status": r.get::<Option<String>, _>("status"),
                                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 6 ── md_list_sponsors ───────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_sponsors",
                "List sponsors in a directory with their commission rate and status. Never exposes \
                 a payment reference. An operator may pass an optional `directory_id`.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = sqlx::query(
                        "SELECT id, business_id, status, commission_rate::float8 AS commission_rate, \
                                created_at \
                           FROM sponsors \
                          WHERE ($1::uuid IS NULL OR directory_id = $1) \
                          ORDER BY created_at DESC LIMIT $2",
                    )
                    .bind(dir)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "business_id": r.get::<Uuid, _>("business_id"),
                                "status": r.get::<String, _>("status"),
                                "commission_rate": r.get::<Option<f64>, _>("commission_rate"),
                                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 7 ── md_list_ad_schedules ───────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_ad_schedules",
                "List ad schedules in a directory with their dates, price and status, plus total \
                 recorded earnings per schedule. An operator may pass an optional `directory_id`.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = sqlx::query(
                        "SELECT id, ad_zone_id, sponsor_id, start_date, end_date, \
                                price_monthly::float8 AS price_monthly, \
                                total_price::float8 AS total_price, status, created_at, \
                                (SELECT COALESCE(SUM(amount), 0)::float8 \
                                   FROM ad_earnings e WHERE e.schedule_id = ad_schedules.id) \
                                    AS total_earned \
                           FROM ad_schedules \
                          WHERE ($1::uuid IS NULL OR directory_id = $1) \
                          ORDER BY created_at DESC LIMIT $2",
                    )
                    .bind(dir)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "ad_zone_id": r.get::<Uuid, _>("ad_zone_id"),
                                "sponsor_id": r.get::<Uuid, _>("sponsor_id"),
                                "start_date": r.get::<Option<DateTime<Utc>>, _>("start_date"),
                                "end_date": r.get::<Option<DateTime<Utc>>, _>("end_date"),
                                "price_monthly": r.get::<Option<f64>, _>("price_monthly"),
                                "total_price": r.get::<Option<f64>, _>("total_price"),
                                "total_earned": r.get::<Option<f64>, _>("total_earned"),
                                "status": r.get::<String, _>("status"),
                                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 8 ── md_list_reviews ────────────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read(
                "md_list_reviews",
                "List reviews (excerpted, untrusted, never the reviewer's email), newest first. A \
                 review is scoped through its business, so a directory key sees only its own \
                 businesses' reviews. An operator may pass an optional `directory_id`.",
            ),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let dir = list_directory(&ctx, &args)?;
                    let rows = sqlx::query(
                        "SELECT r.id, r.business_id, r.rating, r.title, r.reviewer_name, \
                                r.is_verified, r.status, r.created_at, \
                                COALESCE(r.directory_id, b.directory_id) AS directory_id \
                           FROM reviews r JOIN businesses b ON b.id = r.business_id \
                          WHERE ($1::uuid IS NULL OR COALESCE(r.directory_id, b.directory_id) = $1) \
                          ORDER BY r.created_at DESC LIMIT $2",
                    )
                    .bind(dir)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "business_id": r.get::<Option<Uuid>, _>("business_id"),
                                "rating": r.get::<i32, _>("rating"),
                                "title": r.get::<Option<String>, _>("title"),
                                "reviewer_name": r.get::<Option<String>, _>("reviewer_name"),
                                "is_verified": r.get::<Option<bool>, _>("is_verified"),
                                "status": r.get::<Option<String>, _>("status"),
                                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    // 9 ── md_list_seo_submissions ────────────────────────────────────────────
    {
        let pool = pool.clone();
        reg = reg.with_fn(
            ToolSpec::read("md_list_seo_submissions",
                "List search-engine sitemap submission URLs and their index status. Operator only \
                 (submission data is platform-wide, not directory-scoped).",
            )
            .min_role(Role::Admin),
            move |ctx, args| {
                let pool = pool.clone();
                Box::pin(async move {
                    ctx.require_admin()?;
                    let limit = ctx.limit(args.get("limit"), 25, 200);
                    let rows = sqlx::query(
                        "SELECT id, scope, url, first_submitted_at \
                           FROM seo_submission_urls ORDER BY first_submitted_at DESC LIMIT $1",
                    )
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(db_err)?;
                    let items: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.get::<Uuid, _>("id"),
                                "scope": r.get::<Option<String>, _>("scope"),
                                "url": r.get::<Option<String>, _>("url"),
                                "first_submitted_at": r.get::<Option<DateTime<Utc>>, _>("first_submitted_at"),
                            })
                        })
                        .collect();
                    Ok(json!({ "count": items.len(), "items": items }))
                })
            },
        );
    }

    reg
}

// ─────────────────────────────────────────────────────────────────────────────
// Door construction + the axum handler
// ─────────────────────────────────────────────────────────────────────────────

/// Build the door against a live pool. Fails closed: a bad registry or config refuses startup.
pub fn build_door(pool: PgPool) -> Result<Door, mcp_door::transport::DoorError> {
    Door::builder(
        DOOR_NAME,
        env!("CARGO_PKG_VERSION"),
        registry(pool.clone()),
        Arc::new(ApiKeyResolver { pool }),
    )
    .audit(AUDIT_PATH)
    .instructions(
        "Read-only access to the ZaarHub directory network. An operator credential sees every \
         directory; a per-directory credential sees only its own directory's listings, sponsors, \
         ads, submissions, reviews and categories.",
    )
    .build()
}

/// The `/admin/mcp` route handler. The door itself is the credential boundary for this path (it
/// answers 401 with the JSON-RPC unauthorized code), so it is mounted as a PUBLIC route.
pub async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    state.mcp_door.dispatch(&headers, &body).await
}
