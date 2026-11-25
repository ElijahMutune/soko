use axum::{extract::{Path, Query, State}, http::{HeaderMap, StatusCode}, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::{auth::routes::authenticated_user_id, error::AppError, models::product::{CreateProduct, Product}, state::AppState};

#[derive(Debug, Deserialize)]
pub struct ProductQuery {
    pub q: Option<String>,
    pub category_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct ProductList {
    pub items: Vec<Product>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_products(
    State(state): State<AppState>,
    Query(query): Query<ProductQuery>,
) -> Result<Json<ProductList>, AppError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = query.q.unwrap_or_default();

    let items = sqlx::query_as::<_, Product>(
        r#"SELECT id, title, description, price_kes, category_id, image_url,
                  condition, status, created_at
           FROM products
           WHERE status = 'active'
             AND ($1 = '' OR title ILIKE '%' || $1 || '%' OR COALESCE(description, '') ILIKE '%' || $1 || '%')
             AND ($2::uuid IS NULL OR category_id = $2)
           ORDER BY created_at DESC
           LIMIT $3 OFFSET $4"#
    )
    .bind(q)
    .bind(query.category_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(ProductList { items, limit, offset }))
}

pub async fn get_product(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Product>, AppError> {
    let product = sqlx::query_as::<_, Product>(
        r#"SELECT id, title, description, price_kes, category_id, image_url,
                  condition, status, created_at
           FROM products WHERE id = $1 AND status = 'active'"#
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(product))
}

pub async fn create_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateProduct>,
) -> Result<(StatusCode, Json<Product>), AppError> {
    // Derive seller identity from the verified token. Never accept seller_id
    // from the request body, because a client could impersonate another seller.
    let seller_id = authenticated_user_id(&state, &headers).await?;
    payload.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;

    let product = sqlx::query_as::<_, Product>(
        r#"INSERT INTO products
           (seller_id, title, description, price_kes, category_id, image_url, condition, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'active')
           RETURNING id, title, description, price_kes, category_id, image_url,
                     condition, status, created_at"#
    )
    .bind(seller_id)
    .bind(payload.title.trim())
    .bind(payload.description)
    .bind(payload.price_kes)
    .bind(payload.category_id)
    .bind(payload.image_url)
    .bind(payload.condition.trim())
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(product)))
}
