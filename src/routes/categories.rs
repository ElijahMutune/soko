use axum::{extract::State, Json};
use serde::Serialize;
use uuid::Uuid;
use crate::{error::AppError, state::AppState};

#[derive(Serialize, sqlx::FromRow)]
pub struct Category {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

pub async fn list_categories(
    State(state): State<AppState>,
) -> Result<Json<Vec<Category>>, AppError> {
    let categories = sqlx::query_as::<_, Category>(
        "SELECT id, name, slug FROM categories WHERE is_active = TRUE ORDER BY sort_order, name"
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(categories))
}
