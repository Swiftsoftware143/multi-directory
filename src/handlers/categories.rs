use axum::{extract::State, Json};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiResult;

#[derive(Debug, Serialize, FromRow)]
pub struct Category {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub directory_id: Uuid,
}

pub async fn list_all_categories(State(s): State<AppState>) -> ApiResult<Json<Vec<Category>>> {
    let cats = sqlx::query_as::<_, Category>(
        "SELECT id, name, slug, directory_id FROM directory_categories ORDER BY name"
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(cats))
}
