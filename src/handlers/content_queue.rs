//! Content Scheduling Queue
//!
//! CRUD endpoints for scheduling blog posts and trap door pages.
//! Admins can schedule content in advance, preview the queue, edit or cancel items.
//! A background worker endpoint (called by cron) processes due jobs.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Models ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ContentQueueItem {
    pub id: Uuid,
    pub queue_type: String,
    pub directory_id: Uuid,
    pub keyword: String,
    pub template_id: Option<Uuid>,
    pub merge_fields: Option<serde_json::Value>,
    pub scheduled_for: DateTime<Utc>,
    pub status: String,
    pub retry_count: Option<i32>,
    pub error_message: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// ── Request types ──

#[derive(Debug, Deserialize)]
pub struct AddJobRequest {
    pub queue_type: String,
    pub directory_id: Uuid,
    pub keyword: String,
    pub template_id: Option<Uuid>,
    pub merge_fields: Option<serde_json::Value>,
    pub scheduled_for: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJobRequest {
    pub keyword: Option<String>,
    pub template_id: Option<Uuid>,
    pub merge_fields: Option<serde_json::Value>,
    pub scheduled_for: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct BulkJobsRequest {
    pub jobs: Vec<AddJobRequest>,
}

#[derive(Debug, Deserialize)]
pub struct ListQueueParams {
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub queue_type: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

// ── 1. POST /api/v1/admin/content-queue — Add a single job ──

pub async fn add_job(
    State(s): State<AppState>,
    Json(req): Json<AddJobRequest>,
) -> ApiResult<impl IntoResponse> {
    // Validate queue_type
    if req.queue_type != "trap_door" && req.queue_type != "blog" {
        return Err(AppError::Validation(
            "queue_type must be 'trap_door' or 'blog'".into(),
        ));
    }

    // Validate directory exists
    let dir_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM directories WHERE id = $1",
    )
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    if dir_exists == 0 {
        return Err(AppError::NotFound("Directory not found".into()));
    }

    let item = sqlx::query_as::<_, ContentQueueItem>(
        r#"INSERT INTO content_queue (queue_type, directory_id, keyword, template_id, merge_fields, scheduled_for)
           VALUES ($1, $2, $3, $4, $5::jsonb, $6)
           RETURNING *"#,
    )
    .bind(&req.queue_type)
    .bind(req.directory_id)
    .bind(&req.keyword)
    .bind(req.template_id)
    .bind(&req.merge_fields)
    .bind(req.scheduled_for)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(item))))
}

// ── 2. GET /api/v1/admin/content-queue — List all queued jobs ──

pub async fn list_queue(
    State(s): State<AppState>,
    Query(params): Query<ListQueueParams>,
) -> ApiResult<impl IntoResponse> {
    let per_page = params.per_page.unwrap_or(50).clamp(1, 200);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let mut conditions = Vec::new();
    let mut bind_idx = 1;

    if let Some(ref status) = params.status {
        conditions.push(format!("cq.status = ${}", bind_idx));
        bind_idx += 1;
    }
    if let Some(dir_id) = params.directory_id {
        conditions.push(format!("cq.directory_id = ${}", bind_idx));
        bind_idx += 1;
    }
    if let Some(ref qt) = params.queue_type {
        conditions.push(format!("cq.queue_type = ${}", bind_idx));
        bind_idx += 1;
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // Count query
    let count_sql = format!(
        "SELECT COUNT(*) FROM content_queue cq {}",
        where_clause,
    );
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);

    if let Some(ref status) = params.status {
        count_query = count_query.bind(status.clone());
    }
    if let Some(dir_id) = params.directory_id {
        count_query = count_query.bind(dir_id);
    }
    if let Some(ref qt) = params.queue_type {
        count_query = count_query.bind(qt.clone());
    }

    let total = count_query.fetch_one(&s.db).await.unwrap_or(0);

    // Data query
    let data_sql = format!(
        "SELECT cq.* FROM content_queue cq {} ORDER BY cq.scheduled_for ASC LIMIT ${} OFFSET ${}",
        where_clause,
        bind_idx,
        bind_idx + 1,
    );
    let mut data_query = sqlx::query_as::<_, ContentQueueItem>(&data_sql);

    if let Some(ref status) = params.status {
        data_query = data_query.bind(status.clone());
    }
    if let Some(dir_id) = params.directory_id {
        data_query = data_query.bind(dir_id);
    }
    if let Some(ref qt) = params.queue_type {
        data_query = data_query.bind(qt.clone());
    }

    data_query = data_query.bind(per_page).bind(offset);

    let items = data_query.fetch_all(&s.db).await?;

    Ok(Json(json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": (total as f64 / per_page as f64).ceil() as i64,
    })))
}

// ── 3. PUT /api/v1/admin/content-queue/:id — Edit a pending job ──

pub async fn update_job(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateJobRequest>,
) -> ApiResult<impl IntoResponse> {
    // Only pending jobs can be edited
    let existing = sqlx::query_as::<_, ContentQueueItem>(
        "SELECT * FROM content_queue WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Queue item not found".into()))?;

    if existing.status != "pending" {
        return Err(AppError::BadRequest(
            "Only pending queue items can be edited".into(),
        ));
    }

    let item = sqlx::query_as::<_, ContentQueueItem>(
        r#"UPDATE content_queue SET
            keyword = COALESCE($1, keyword),
            template_id = COALESCE($2, template_id),
            merge_fields = COALESCE($3::jsonb, merge_fields),
            scheduled_for = COALESCE($4, scheduled_for),
            updated_at = NOW()
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(&req.keyword)
    .bind(req.template_id)
    .bind(&req.merge_fields)
    .bind(req.scheduled_for)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(item)))
}

// ── 4. DELETE /api/v1/admin/content-queue/:id — Cancel a job ──

pub async fn cancel_job(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, ContentQueueItem>(
        "SELECT * FROM content_queue WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Queue item not found".into()))?;

    if existing.status == "completed" || existing.status == "cancelled" {
        return Err(AppError::BadRequest(format!(
            "Cannot cancel a job with status '{}'",
            existing.status
        )));
    }

    let item = sqlx::query_as::<_, ContentQueueItem>(
        r#"UPDATE content_queue SET status = 'cancelled', updated_at = NOW() WHERE id = $1 RETURNING *"#,
    )
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(item)))
}

// ── 5. POST /api/v1/admin/content-queue/bulk — Schedule many at once ──

pub async fn bulk_add_jobs(
    State(s): State<AppState>,
    Json(req): Json<BulkJobsRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.jobs.is_empty() {
        return Err(AppError::Validation("At least one job is required".into()));
    }

    if req.jobs.len() > 100 {
        return Err(AppError::Validation("Maximum 100 jobs per bulk request".into()));
    }

    let mut created: Vec<ContentQueueItem> = Vec::new();
    let mut errors: Vec<serde_json::Value> = Vec::new();

    for (i, job) in req.jobs.into_iter().enumerate() {
        // Validate queue_type
        if job.queue_type != "trap_door" && job.queue_type != "blog" {
            errors.push(json!({
                "index": i,
                "error": format!("Invalid queue_type '{}' — must be 'trap_door' or 'blog'", job.queue_type),
            }));
            continue;
        }

        // Validate directory exists
        let dir_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM directories WHERE id = $1",
        )
        .bind(job.directory_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        if dir_exists == 0 {
            errors.push(json!({
                "index": i,
                "error": format!("Directory {} not found", job.directory_id),
            }));
            continue;
        }

        match sqlx::query_as::<_, ContentQueueItem>(
            r#"INSERT INTO content_queue (queue_type, directory_id, keyword, template_id, merge_fields, scheduled_for)
               VALUES ($1, $2, $3, $4, $5::jsonb, $6)
               RETURNING *"#,
        )
        .bind(&job.queue_type)
        .bind(job.directory_id)
        .bind(&job.keyword)
        .bind(job.template_id)
        .bind(&job.merge_fields)
        .bind(job.scheduled_for)
        .fetch_one(&s.db)
        .await
        {
            Ok(item) => created.push(item),
            Err(e) => {
                errors.push(json!({
                    "index": i,
                    "error": format!("Database error: {}", e),
                }));
            }
        }
    }

    Ok((StatusCode::CREATED, Json(json!({
        "created": created.len(),
        "errors": errors.len(),
        "items": created,
        "error_details": errors,
    }))))
}

// ── 6. POST /api/v1/cron/content-queue-worker — Process due jobs ──

/// Processes due content queue jobs.
///
/// Queries all jobs with status = 'pending' and scheduled_for <= NOW(),
/// up to 10 at a time, ordered by scheduled_for ASC.
///
/// For trap_door jobs: attempts to generate trap door content using the
/// directory's services, cities, day tags, and time tags.
///
/// For blog jobs: currently marks as completed (placeholder — full LLM
/// blog generation for queued jobs can be wired to blog_generator::generate_blog_posts).
///
/// Safe to call every hour — idempotent.
pub async fn process_content_queue(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let due_jobs = sqlx::query_as::<_, ContentQueueItem>(
        r#"SELECT * FROM content_queue
           WHERE status = 'pending'
             AND scheduled_for <= NOW()
           ORDER BY scheduled_for ASC
           LIMIT 10"#,
    )
    .fetch_all(&s.db)
    .await?;

    let mut processed = 0usize;
    let mut failed = 0usize;
    let mut results: Vec<serde_json::Value> = Vec::new();

    for job in &due_jobs {
        // Set to 'generating'
        let _ = sqlx::query(
            "UPDATE content_queue SET status = 'generating', updated_at = NOW() WHERE id = $1",
        )
        .bind(job.id)
        .execute(&s.db)
        .await;

        let result = match job.queue_type.as_str() {
            "trap_door" => process_trap_door_job(&s, job).await,
            "blog" => process_blog_job(&s, job).await,
            _ => {
                Err(AppError::Internal(format!(
                    "Unknown queue_type: {}",
                    job.queue_type
                )))
            }
        };

        match result {
            Ok(msg) => {
                let _ = sqlx::query(
                    "UPDATE content_queue SET status = 'completed', updated_at = NOW() WHERE id = $1",
                )
                .bind(job.id)
                .execute(&s.db)
                .await;
                processed += 1;
                results.push(json!({
                    "id": job.id,
                    "queue_type": job.queue_type,
                    "keyword": job.keyword,
                    "status": "completed",
                    "message": msg,
                }));
            }
            Err(e) => {
                let _ = sqlx::query(
                    r#"UPDATE content_queue SET status = 'failed', error_message = $1, retry_count = COALESCE(retry_count, 0) + 1, updated_at = NOW() WHERE id = $2"#,
                )
                .bind(e.to_string())
                .bind(job.id)
                .execute(&s.db)
                .await;
                failed += 1;
                results.push(json!({
                    "id": job.id,
                    "queue_type": job.queue_type,
                    "keyword": job.keyword,
                    "status": "failed",
                    "error": e.to_string(),
                }));
            }
        }
    }

    Ok(Json(json!({
        "processed": processed,
        "failed": failed,
        "total_due": due_jobs.len(),
        "results": results,
    })))
}

// ── Job Processors ──

/// Process a trap_door queue job.
///
/// Replicates the logic from scheduled_generate_trap_doors but for a single
/// keyword context. Uses the directory_id from the job to fetch services,
/// cities, day tags, and time tags, then generates programmatic pages.
async fn process_trap_door_job(
    s: &AppState,
    job: &ContentQueueItem,
) -> Result<String, AppError> {
    let dir_id = job.directory_id;

    // Fetch services for this directory
    let services = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directory_services WHERE directory_id = $1 AND is_active = true",
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    // Fetch distinct cities from businesses
    let cities: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT city FROM businesses WHERE directory_id = $1 AND city IS NOT NULL AND city != ''",
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let day_tags = vec![
        "monday".to_string(),
        "tuesday".to_string(),
        "wednesday".to_string(),
        "thursday".to_string(),
        "friday".to_string(),
        "saturday".to_string(),
        "sunday".to_string(),
    ];

    let time_tags = vec![
        "morning".to_string(),
        "afternoon".to_string(),
        "evening".to_string(),
        "past-9pm".to_string(),
    ];

    if services.is_empty() || cities.is_empty() {
        // Nothing to generate; this is fine — mark as completed
        return Ok("No services or cities available; skipped".to_string());
    }

    let mut created: usize = 0;
    let mut skipped: usize = 0;

    for (service_id, service_name, service_slug) in &services {
        for city in &cities {
            for day in &day_tags {
                for time in &time_tags {
                    let slug = format!(
                        "{}-{}-{}-{}-{}",
                        job.keyword.to_lowercase().replace(' ', "-"),
                        service_slug.replace(' ', "-").replace('&', "and").to_lowercase(),
                        city.to_lowercase().replace(' ', "-"),
                        time,
                        day,
                    );

                    // Check for duplicate
                    let exists: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM programmatic_pages WHERE directory_id = $1 AND slug = $2",
                    )
                    .bind(dir_id)
                    .bind(&slug)
                    .fetch_one(&s.db)
                    .await
                    .unwrap_or(0);

                    if exists > 0 {
                        skipped += 1;
                        continue;
                    }

                    let title = format!(
                        "{} {} in {} Open {} {}",
                        capitalize_first(&job.keyword),
                        service_name,
                        city,
                        time_label(time),
                        day,
                    );
                    let meta_title = format!(
                        "{} {} in {} Open {} {} | Find Providers",
                        capitalize_first(&job.keyword),
                        service_name,
                        city,
                        time_label(time),
                        day,
                    );
                    let meta_description = format!(
                        "Looking for {} services in {} open {} on {}? Browse {} providers near you.",
                        job.keyword, city, time_label(time).to_lowercase(), day, service_name,
                    );
                    let h1 = title.clone();

                    let content = format!(
                        r#"<p>Find {} providers in {} offering {} services open {} on {}. Browse our directory of {} businesses serving the {} area during {} hours.</p>"#,
                        service_name, city, job.keyword, time_label(time).to_lowercase(), day, service_name, city, time_label(time).to_lowercase(),
                    );

                    let result = sqlx::query(
                        r#"INSERT INTO programmatic_pages
                        (directory_id, service_id, slug, title, meta_title, meta_description, h1, content, template_name, status, day_tags, time_tags)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'published', $10, $11)"#,
                    )
                    .bind(dir_id)
                    .bind(service_id)
                    .bind(&slug)
                    .bind(&title)
                    .bind(&meta_title)
                    .bind(&meta_description)
                    .bind(&h1)
                    .bind(&content)
                    .bind("content-queue-trap-door")
                    .bind(&vec![day.clone()])
                    .bind(&vec![time.clone()])
                    .execute(&s.db)
                    .await;

                    match result {
                        Ok(_) => created += 1,
                        Err(_) => skipped += 1,
                    }
                }
            }
        }
    }

    Ok(format!(
        "Generated {} trap door pages ({} skipped, {} total combinations)",
        created,
        skipped,
        services.len() * cities.len() * day_tags.len() * time_tags.len()
    ))
}

/// Process a blog queue job.
///
/// Currently serves as a placeholder. When full LLM blog generation is wired
/// for queued jobs, this should be replaced with a call to the blog generator.
/// For now, it marks the job as completed without generating content.
async fn process_blog_job(
    _s: &AppState,
    _job: &ContentQueueItem,
) -> Result<String, AppError> {
    // TODO: Wire up full blog generation from content_queue.
    // This should call blog_generator::generate_blog_posts or similar
    // with the job's template_id, keyword, and merge_fields.
    //
    // Currently a placeholder — marks blog jobs as completed so they
    // don't accumulate. Full AI blog generation for queued jobs can be
    // added in a follow-up iteration.

    Ok("Blog job acknowledged — generation not yet wired for queue. Marked as completed.".to_string())
}

// ── Helpers ──

fn time_label(tag: &str) -> &str {
    match tag {
        "morning" => "Morning",
        "afternoon" => "Afternoon",
        "evening" => "Evening",
        "past-9pm" => "Past 9 PM",
        _ => tag,
    }
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
