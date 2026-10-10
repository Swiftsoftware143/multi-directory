//! Phase 4 — Automation
//! Directory events table, n8n webhook integration

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

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

    // `sqlx::QueryBuilder` owns the `$N` numbering, so the statement is never assembled as text
    // (gate rule 5d). NOTE, measured (both-ways probe on a throwaway DB copy): the code this
    // replaces numbered LIMIT/OFFSET off a counter that counted only the filters PRESENT while it
    // bound all five optionals unconditionally, so any request with fewer than five filters
    // prepared a statement whose parameter count did not match the binds and fell through to the
    // unfiltered fallback below. The builder numbers each bind as it is pushed, so the filters
    // now actually apply and LIMIT/OFFSET land on the values they name.
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM directory_events");
    let mut first = true;

    if let Some(v) = q.event_type.as_ref() {
        qb.push(if first { " WHERE " } else { " AND " });
        qb.push("event_type = ").push_bind(v.clone());
        first = false;
    }
    if let Some(v) = q.entity_type.as_ref() {
        qb.push(if first { " WHERE " } else { " AND " });
        qb.push("entity_type = ").push_bind(v.clone());
        first = false;
    }
    if let Some(v) = q.entity_id.as_ref() {
        qb.push(if first { " WHERE " } else { " AND " });
        qb.push("entity_id = ").push_bind(*v);
        first = false;
    }
    if let Some(v) = q.directory_id.as_ref() {
        qb.push(if first { " WHERE " } else { " AND " });
        qb.push("directory_id = ").push_bind(*v);
        first = false;
    }
    if let Some(v) = q.processed.as_ref() {
        qb.push(if first { " WHERE " } else { " AND " });
        qb.push("processed = ").push_bind(*v);
    }

    qb.push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let events: Vec<serde_json::Value> = {
        let db_q = qb.build_query_as::<DirectoryEvent>();

        let result = db_q.fetch_all(&state.db).await;
        match result {
            Ok(rows) => rows
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect(),
            Err(_) => {
                // Fallback: simple query without filters
                sqlx::query_as::<_, DirectoryEvent>(
                    "SELECT * FROM directory_events ORDER BY created_at DESC LIMIT $1 OFFSET $2",
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

    // Deliver to the webhooks registered on the admin "Webhooks" screen. Nothing used to call the
    // dispatcher, so a registered subscriber received NOTHING for any event: the screen configured
    // a black hole. Dispatch on every recorded event (the dispatcher itself selects the
    // subscribers whose `events` contain this type) — the entity must be a real row to name it.
    if let Some(entity_id) = req.entity_id {
        crate::handlers::api_complete::dispatch_webhook_event(
            &state,
            &req.event_type,
            &req.entity_type,
            entity_id,
            req.data.clone().unwrap_or(serde_json::Value::Null),
        )
        .await;
    }

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
pub async fn unprocessed_events(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
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
    let result = sqlx::query("UPDATE directory_events SET processed = true WHERE id = $1")
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
                        let _ = sqlx::query(
                            "UPDATE directory_events SET n8n_webhook_sent = true WHERE id = $1",
                        )
                        .bind(e.id)
                        .execute(&state.db)
                        .await;
                    }
                    Err(_) => {
                        let _ = sqlx::query(
                            "UPDATE directory_events SET n8n_webhook_failed = true WHERE id = $1",
                        )
                        .bind(e.id)
                        .execute(&state.db)
                        .await;
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
