use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub price_kes: i64,
    pub category_id: Option<Uuid>,
    pub image_url: Option<String>,
    pub condition: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateProduct {
    #[validate(length(min = 3, max = 120))]
    pub title: String,
    #[validate(length(max = 3000))]
    pub description: Option<String>,
    #[validate(range(min = 1, max = 100_000_000))]
    pub price_kes: i64,
    pub category_id: Option<Uuid>,
    #[validate(url)]
    pub image_url: Option<String>,
    #[validate(length(min = 2, max = 30))]
    pub condition: String,
}
