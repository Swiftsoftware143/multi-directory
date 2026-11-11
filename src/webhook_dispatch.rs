//! Webhook dispatch drain.
//!
//! The admin "API Access & Webhooks" screen registers a URL plus the events the operator wants.
//! Every change the platform records lands in `directory_events` (database triggers on businesses,
//! directories, categories; the review trigger added by migration 177; the booking and contact
//! paths). This task is the ONE delivery point: it claims undispatched rows and hands each one to
//! `dispatch_webhook_event`, which owns subscriber matching, signing, retries and the delivery log.
//!
//! Why a background drain rather than a call in every handler: the rows are written by the DATABASE
//! (an import creates listings straight through the trigger), so a handler-level call would miss
//! exactly the changes an operator most wants to hear about. One drain covers them all, and
//! `webhook_dispatched_at` makes delivery exactly-once across restarts.
//!
//! Not a queue to grow forever: rows older than an hour are stamped and skipped, so an outage can
//! never turn into a burst of stale deliveries when the app comes back.

use std::time::Duration;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::handlers::api_complete::dispatch_webhook_event;

/// How often the drain looks for new events.
const TICK: Duration = Duration::from_secs(5);
/// Rows claimed per tick — a burst (a bulk import) is worked off over several ticks.
const BATCH: i64 = 100;
/// An event older than this is history, not news. Stamp it and move on.
const MAX_AGE_MINUTES: i64 = 60;

pub fn start_webhook_dispatch(db: PgPool) {
    tokio::spawn(async move {
        // Let the app finish booting; the migrations that create the column run before this is
        // called, but the HTTP server is not needed for anything here.
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match drain_once(&db).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("[webhooks] dispatched {} event(s)", n),
                Err(e) => tracing::warn!("[webhooks] drain failed: {}", e),
            }
        }
    });
}

/// Deliver every undispatched event once. Returns how many rows were handed to the dispatcher.
pub async fn drain_once(db: &PgPool) -> Result<usize, sqlx::Error> {
    // Anything that went stale while the app was down is stamped, never delivered.
    sqlx::query(
        "UPDATE directory_events SET webhook_dispatched_at = NOW()
          WHERE webhook_dispatched_at IS NULL
            AND created_at < NOW() - ($1 || ' minutes')::interval",
    )
    .bind(MAX_AGE_MINUTES)
    .execute(db)
    .await?;

    let rows = sqlx::query(
        "SELECT id, event_type, entity_type, entity_id, COALESCE(data, 'null'::jsonb) AS data
           FROM directory_events
          WHERE webhook_dispatched_at IS NULL
          ORDER BY created_at
          LIMIT $1",
    )
    .bind(BATCH)
    .fetch_all(db)
    .await?;

    let mut delivered = 0usize;
    for row in rows {
        let id: Uuid = row.get("id");
        let event_type: String = row.get("event_type");
        let entity_type: String = row.get("entity_type");
        let entity_id: Option<Uuid> = row.try_get("entity_id").ok();
        let data: serde_json::Value = row.try_get("data").unwrap_or(serde_json::Value::Null);

        // A row always carries an entity id today, but an event with none is still stamped so it
        // cannot be retried for ever.
        if let Some(entity_id) = entity_id {
            dispatch_webhook_event(db, &event_type, &entity_type, entity_id, data).await;
            delivered += 1;
        }

        sqlx::query("UPDATE directory_events SET webhook_dispatched_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(db)
            .await?;
    }

    Ok(delivered)
}
