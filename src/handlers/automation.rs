//! Phase 4 — Automation
//! Directory events table, n8n webhook integration

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Directory Events ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DirectoryEvent {
    pub id: Uuid,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub data: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub processed: Option<bool>,
    pub n8n_webhook_sent: Option<bool>,
    pub n8n_webhook_failed: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub data: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    pub event_type: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub processed: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// GET /api/v1/events — list directory events
pub async fn list_events(
    State(state): State<AppState>,
    Query(q): Query<ListEventsQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    // Build dynamic query
    let mut where_clauses: Vec<String> = Vec::new();
    let mut param_idx = 0u32;

    let mut query_str = "SELECT * FROM directory_events".to_string();

    if q.event_type.is_some() {
        param_idx += 1;
        where_clauses.push(format!("event_type = ${}", param_idx));
    }
    if q.entity_type.is_some() {
        param_idx += 1;
        where_clauses.push(format!("entity_type = ${}", param_idx));
    }
    if q.entity_id.is_some() {
        param_idx += 1;
        where_clauses.push(format!("entity_id = ${}", param_idx));
    }
    if q.directory_id.is_some() {
        param_idx += 1;
        where_clauses.push(format!("directory_id = ${}", param_idx));
    }
    if q.processed.is_some() {
        param_idx += 1;
        where_clauses.push(format!("processed = ${}", param_idx));
    }

    if !where_clauses.is_empty() {
        query_str.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
    }

    query_str.push_str(" ORDER BY created_at DESC");
    query_str.push_str(&format!(" LIMIT ${} OFFSET ${}", param_idx + 1, param_idx + 2));

    // Build the query manually since sqlx doesn't support dynamic query building well
    let events: Vec<serde_json::Value> = {
        let db_q = sqlx::query_as::<_, DirectoryEvent>(&query_str)
            .bind(&q.event_type)
            .bind(&q.entity_type)
            .bind(q.entity_id)
            .bind(q.directory_id)
            .bind(q.processed)
            .bind(limit)
            .bind(offset);

        let result = db_q.fetch_all(&state.db).await;
        match result {
            Ok(rows) => rows.into_iter().map(|e| serde_json::to_value(e).unwrap_or_default()).collect(),
            Err(_) => {
                // Fallback: simple query without filters
                sqlx::query_as::<_, DirectoryEvent>(
                    "SELECT * FROM directory_events ORDER BY created_at DESC LIMIT $1 OFFSET $2"
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&state.db)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect()
            }
        }
    };

    // Get total count
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM directory_events")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

    Ok(Json(serde_json::json!({
        "events": events,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

/// POST /api/v1/events — create a directory event
pub async fn create_event(
    State(state): State<AppState>,
    Json(req): Json<CreateEventRequest>,
) -> ApiResult<impl IntoResponse> {
    let event = sqlx::query_as::<_, DirectoryEvent>(
        "INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, tenant_id, actor_id, data, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb) RETURNING *"
    )
    .bind(&req.event_type)
    .bind(&req.entity_type)
    .bind(req.entity_id)
    .bind(req.directory_id)
    .bind(req.tenant_id)
    .bind(req.actor_id)
    .bind(&req.data)
    .bind(&req.metadata)
    .fetch_one(&state.db)
    .await?;

    // Try to forward to n8n if configured
    let n8n_url = std::env::var("N8N_WEBHOOK_URL").ok();
    if let Some(url) = n8n_url {
        let event_clone = event.id;
        tokio::spawn(async move {
            let payload = serde_json::json!({
                "event_id": event_clone,
                "event_type": req.event_type,
                "entity_type": req.entity_type,
                "entity_id": req.entity_id,
                "directory_id": req.directory_id,
                "actor_id": req.actor_id,
                "data": req.data,
                "metadata": req.metadata,
                "timestamp": Utc::now().to_rfc3339(),
            });

            match reqwest::Client::new()
                .post(&url)
                .json(&payload)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status().is_success();
                    let _ = sqlx::query(
                        "UPDATE directory_events SET n8n_webhook_sent = $1, n8n_webhook_failed = $2 WHERE id = $3"
                    )
                    .bind(status)
                    .bind(!status)
                    .bind(event_clone)
                    .execute(&state.db)
                    .await;
                }
                Err(e) => {
                    tracing::warn!("Failed to forward event to n8n: {}", e);
                    let _ = sqlx::query(
                        "UPDATE directory_events SET n8n_webhook_sent = false, n8n_webhook_failed = true WHERE id = $1"
                    )
                    .bind(event_clone)
                    .execute(&state.db)
                    .await;
                }
            }
        });
    }

    Ok((StatusCode::CREATED, Json(serde_json::json!(event))))
}

/// GET /api/v1/events/unprocessed — get events not yet processed by n8n
pub async fn unprocessed_events(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let events = sqlx::query_as::<_, DirectoryEvent>(
        "SELECT * FROM directory_events WHERE n8n_webhook_sent = false OR n8n_webhook_sent IS NULL ORDER BY created_at ASC LIMIT 100"
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!(events)))
}

/// POST /api/v1/events/:id/process — mark an event as processed
pub async fn mark_event_processed(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query(
        "UPDATE directory_events SET processed = true WHERE id = $1"
    )
    .bind(id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Event not found".to_string()));
    }
    Ok(Json(serde_json::json!({ "status": "processed", "id": id })))
}

// ── n8n Webhook Receiver ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct N8nWebhookPayload {
    pub action: Option<String>,
    pub event_type: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub data: Option<serde_json::Value>,
}

/// POST /api/v1/n8n/webhook — receive webhook from n8n
pub async fn n8n_webhook_receiver(
    Json(payload): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    tracing::info!("Received n8n webhook: {:?}", payload);

    Ok(Json(serde_json::json!({
        "status": "received",
        "message": "Event forwarded to Multi-Directory",
        "timestamp": Utc::now().to_rfc3339(),
    })))
}

/// GET /api/v1/n8n/health — n8n health check endpoint
pub async fn n8n_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "multidirectory-n8n-bridge",
        "timestamp": Utc::now().to_rfc3339(),
    }))
}

// ── Event Recording: convenience function for internal use ────────────────────

/// Record an event and optionally dispatch to matching webhooks + n8n
pub async fn record_event(
    state: &AppState,
    event_type: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    directory_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    actor_id: Option<Uuid>,
    data: Option<serde_json::Value>,
) -> Uuid {
    let event = sqlx::query_as::<_, DirectoryEvent>(
        "INSERT INTO directory_events (event_type, entity_type, entity_id, directory_id, tenant_id, actor_id, data)
         VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb) RETURNING id"
    )
    .bind(event_type)
    .bind(entity_type)
    .bind(entity_id)
    .bind(directory_id)
    .bind(tenant_id)
    .bind(actor_id)
    .bind(&data)
    .fetch_one(&state.db)
    .await;

    match event {
        Ok(e) => {
            // Forward to n8n
            let n8n_url = std::env::var("N8N_WEBHOOK_URL").ok();
            if let Some(url) = n8n_url {
                let payload = serde_json::json!({
                    "event_id": e.id,
                    "event_type": event_type,
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "directory_id": directory_id,
                    "tenant_id": tenant_id,
                    "actor_id": actor_id,
                    "data": data,
                    "timestamp": Utc::now().to_rfc3339(),
                });

                match reqwest::Client::new()
                    .post(&url)
                    .json(&payload)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(_) => {
                        let _ = sqlx::query("UPDATE directory_events SET n8n_webhook_sent = true WHERE id = $1")
                            .bind(e.id).execute(&state.db).await;
                    }
                    Err(_) => {
                        let _ = sqlx::query("UPDATE directory_events SET n8n_webhook_failed = true WHERE id = $1")
                            .bind(e.id).execute(&state.db).await;
                    }
                }
            }

            e.id
        }
        Err(db_err) => {
            tracing::error!("Failed to record event: {}", db_err);
            Uuid::nil()
        }
    }
}
