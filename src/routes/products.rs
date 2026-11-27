use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use validator::Validate;

use crate::{
    auth::routes::authenticated_user_id,
    error::AppError,
    models::product::{CreateProduct, Product},
    state::AppState,
};

const MAX_PRODUCT_IMAGES: usize = 8;

#[derive(Debug, Deserialize)]
pub struct ProductQuery {
    pub q: Option<String>,
    pub category_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ProductImage {
    pub id: Uuid,
    pub image_url: String,
    pub mime_type: String,
    pub byte_size: i64,
    pub sort_order: i16,
    pub is_primary: bool,
}

#[derive(Debug, Serialize)]
pub struct ProductResponse {
    #[serde(flatten)]
    pub product: Product,
    pub images: Vec<ProductImage>,
}

#[derive(Debug, FromRow)]
struct ProductImageRow {
    product_id: Uuid,
    id: Uuid,
    image_url: String,
    mime_type: String,
    byte_size: i64,
    sort_order: i16,
    is_primary: bool,
}

#[derive(Debug, FromRow)]
struct OwnedImageMeta {
    id: Uuid,
    public_url: String,
}

#[derive(Serialize)]
pub struct ProductList {
    pub items: Vec<ProductResponse>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Deserialize)]
pub struct ProductImageIdsRequest {
    pub image_ids: Vec<Uuid>,
}

/// The supplied order becomes the gallery order; the first image is the cover.
#[derive(Debug, Deserialize)]
pub struct ReorderProductImagesRequest {
    pub image_ids: Vec<Uuid>,
}

pub async fn list_products(
    State(state): State<AppState>,
    Query(query): Query<ProductQuery>,
) -> Result<Json<ProductList>, AppError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = query.q.unwrap_or_default();

    let products = sqlx::query_as::<_, Product>(
        r#"SELECT id, title, description, price_kes, category_id, image_url,
                  condition, status, created_at
           FROM products
           WHERE status = 'active'
             AND ($1 = '' OR title ILIKE '%' || $1 || '%' OR COALESCE(description, '') ILIKE '%' || $1 || '%')
             AND ($2::uuid IS NULL OR category_id = $2)
           ORDER BY created_at DESC
           LIMIT $3 OFFSET $4"#,
    )
    .bind(q)
    .bind(query.category_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let items = attach_gallery_images(&state, products).await?;
    Ok(Json(ProductList { items, limit, offset }))
}

pub async fn get_product(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProductResponse>, AppError> {
    let product = sqlx::query_as::<_, Product>(
        r#"SELECT id, title, description, price_kes, category_id, image_url,
                  condition, status, created_at
           FROM products WHERE id = $1 AND status = 'active'"#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut products = attach_gallery_images(&state, vec![product]).await?;
    let response = products.pop().ok_or(AppError::NotFound)?;
    Ok(Json(response))
}

pub async fn create_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateProduct>,
) -> Result<(StatusCode, Json<ProductResponse>), AppError> {
    let seller_id = authenticated_user_id(&state, &headers).await?;
    payload.validate().map_err(|error| AppError::BadRequest(error.to_string()))?;

    let image_ids = collect_image_ids(payload.image_id, payload.image_ids.clone())?;
    let image_meta = fetch_owned_images(&state, seller_id, &image_ids).await?;
    let primary_image_url = image_ids
        .first()
        .and_then(|id| image_meta.iter().find(|row| row.id == *id))
        .map(|row| row.public_url.clone());
    let image_url = primary_image_url.or_else(|| payload.image_url.clone());

    let mut tx = state.db.begin().await?;
    let product = sqlx::query_as::<_, Product>(
        r#"INSERT INTO products
           (seller_id, title, description, price_kes, category_id, image_url, condition, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'active')
           RETURNING id, title, description, price_kes, category_id, image_url,
                     condition, status, created_at"#,
    )
    .bind(seller_id)
    .bind(payload.title.trim())
    .bind(payload.description)
    .bind(payload.price_kes)
    .bind(payload.category_id)
    .bind(image_url)
    .bind(payload.condition.trim())
    .fetch_one(&mut *tx)
    .await?;

    for (index, image_id) in image_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO product_images (product_id, uploaded_image_id, sort_order, is_primary) VALUES ($1, $2, $3, $4)",
        )
        .bind(product.id)
        .bind(image_id)
        .bind(index as i16)
        .bind(index == 0)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let mut responses = attach_gallery_images(&state, vec![product]).await?;
    let response = responses.pop().ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Adds more images to an existing listing owned by the authenticated seller.
pub async fn add_product_images(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(product_id): Path<Uuid>,
    Json(input): Json<ProductImageIdsRequest>,
) -> Result<Json<ProductResponse>, AppError> {
    let seller_id = authenticated_user_id(&state, &headers).await?;
    validate_image_id_list(&input.image_ids, false)?;

    let mut tx = state.db.begin().await?;
    lock_seller_product(&mut tx, product_id, seller_id).await?;

    let existing_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT uploaded_image_id FROM product_images WHERE product_id = $1 ORDER BY sort_order",
    )
    .bind(product_id)
    .fetch_all(&mut *tx)
    .await?;
    if existing_ids.len() + input.image_ids.len() > MAX_PRODUCT_IMAGES {
        return Err(AppError::BadRequest("A product can have at most 8 images".into()));
    }
    if input.image_ids.iter().any(|id| existing_ids.contains(id)) {
        return Err(AppError::BadRequest("An image is already attached to this product".into()));
    }

    let owned = fetch_owned_images(&state, seller_id, &input.image_ids).await?;
    let owned_ids: HashSet<Uuid> = owned.iter().map(|row| row.id).collect();
    if input.image_ids.iter().any(|id| !owned_ids.contains(id)) {
        return Err(AppError::NotFound);
    }

    let first_image = existing_ids.is_empty();
    for (offset, image_id) in input.image_ids.iter().enumerate() {
        let position = existing_ids.len() + offset;
        sqlx::query(
            "INSERT INTO product_images (product_id, uploaded_image_id, sort_order, is_primary) VALUES ($1, $2, $3, $4)",
        )
        .bind(product_id)
        .bind(image_id)
        .bind(position as i16)
        .bind(first_image && offset == 0)
        .execute(&mut *tx)
        .await?;
    }
    sync_primary_image_url(&mut tx, product_id).await?;
    tx.commit().await?;

    let product = get_owned_product(&state, product_id, seller_id).await?;
    let mut responses = attach_gallery_images(&state, vec![product]).await?;
    Ok(Json(responses.pop().ok_or(AppError::NotFound)?))
}

/// Removes one image and automatically promotes the first remaining image.
pub async fn remove_product_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((product_id, image_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ProductResponse>, AppError> {
    let seller_id = authenticated_user_id(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    lock_seller_product(&mut tx, product_id, seller_id).await?;

    let deleted = sqlx::query(
        "DELETE FROM product_images WHERE product_id = $1 AND uploaded_image_id = $2",
    )
    .bind(product_id)
    .bind(image_id)
    .execute(&mut *tx)
    .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    normalize_gallery_order(&mut tx, product_id).await?;
    sync_primary_image_url(&mut tx, product_id).await?;
    tx.commit().await?;

    let product = get_owned_product(&state, product_id, seller_id).await?;
    let mut responses = attach_gallery_images(&state, vec![product]).await?;
    Ok(Json(responses.pop().ok_or(AppError::NotFound)?))
}

/// Reorders all gallery images. The first ID becomes the primary/cover image.
pub async fn reorder_product_images(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(product_id): Path<Uuid>,
    Json(input): Json<ReorderProductImagesRequest>,
) -> Result<Json<ProductResponse>, AppError> {
    let seller_id = authenticated_user_id(&state, &headers).await?;
    validate_image_id_list(&input.image_ids, true)?;

    let mut tx = state.db.begin().await?;
    lock_seller_product(&mut tx, product_id, seller_id).await?;
    let existing_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT uploaded_image_id FROM product_images WHERE product_id = $1 ORDER BY sort_order",
    )
    .bind(product_id)
    .fetch_all(&mut *tx)
    .await?;

    let requested: HashSet<Uuid> = input.image_ids.iter().copied().collect();
    let existing: HashSet<Uuid> = existing_ids.iter().copied().collect();
    if requested != existing || requested.len() != input.image_ids.len() {
        return Err(AppError::BadRequest(
            "image_ids must contain every image currently attached to this product exactly once".into(),
        ));
    }

    // Move rows out of the 0-7 range first to avoid unique(product_id, sort_order)
    // collisions while positions are being reassigned.
    sqlx::query("UPDATE product_images SET sort_order = sort_order + 8, is_primary = FALSE WHERE product_id = $1")
        .bind(product_id)
        .execute(&mut *tx)
        .await?;
    for (index, image_id) in input.image_ids.iter().enumerate() {
        sqlx::query("UPDATE product_images SET sort_order = $3 WHERE product_id = $1 AND uploaded_image_id = $2")
            .bind(product_id)
            .bind(image_id)
            .bind(index as i16)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(primary_id) = input.image_ids.first() {
        sqlx::query("UPDATE product_images SET is_primary = TRUE WHERE product_id = $1 AND uploaded_image_id = $2")
            .bind(product_id)
            .bind(primary_id)
            .execute(&mut *tx)
            .await?;
    }
    sync_primary_image_url(&mut tx, product_id).await?;
    tx.commit().await?;

    let product = get_owned_product(&state, product_id, seller_id).await?;
    let mut responses = attach_gallery_images(&state, vec![product]).await?;
    Ok(Json(responses.pop().ok_or(AppError::NotFound)?))
}

fn collect_image_ids(legacy_image_id: Option<Uuid>, image_ids: Option<Vec<Uuid>>) -> Result<Vec<Uuid>, AppError> {
    let mut ids = image_ids.unwrap_or_default();
    if let Some(legacy_id) = legacy_image_id {
        if ids.is_empty() {
            ids.push(legacy_id);
        } else if !ids.contains(&legacy_id) {
            return Err(AppError::BadRequest("Use image_id or image_ids, not both".into()));
        }
    }
    validate_image_id_list(&ids, true)?;
    Ok(ids)
}

fn validate_image_id_list(ids: &[Uuid], allow_empty: bool) -> Result<(), AppError> {
    if (!allow_empty && ids.is_empty()) || ids.len() > MAX_PRODUCT_IMAGES {
        return Err(AppError::BadRequest("Provide between 1 and 8 image IDs".into()));
    }
    let unique: HashSet<Uuid> = ids.iter().copied().collect();
    if unique.len() != ids.len() {
        return Err(AppError::BadRequest("Duplicate image IDs are not allowed".into()));
    }
    Ok(())
}

async fn fetch_owned_images(
    state: &AppState,
    owner_id: Uuid,
    image_ids: &[Uuid],
) -> Result<Vec<OwnedImageMeta>, AppError> {
    if image_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, OwnedImageMeta>(
        "SELECT id, public_url FROM uploaded_images WHERE owner_id = $1 AND id = ANY($2) AND storage_provider = 'local'",
    )
    .bind(owner_id)
    .bind(image_ids)
    .fetch_all(&state.db)
    .await?;
    if rows.len() != image_ids.len() {
        return Err(AppError::NotFound);
    }
    Ok(rows)
}

async fn attach_gallery_images(
    state: &AppState,
    products: Vec<Product>,
) -> Result<Vec<ProductResponse>, AppError> {
    if products.is_empty() {
        return Ok(Vec::new());
    }
    let product_ids: Vec<Uuid> = products.iter().map(|product| product.id).collect();
    let rows = sqlx::query_as::<_, ProductImageRow>(
        r#"SELECT pi.product_id, ui.id, ui.public_url AS image_url, ui.mime_type,
                  ui.byte_size, pi.sort_order, pi.is_primary
           FROM product_images pi
           JOIN uploaded_images ui ON ui.id = pi.uploaded_image_id
           WHERE pi.product_id = ANY($1)
           ORDER BY pi.product_id, pi.sort_order"#,
    )
    .bind(&product_ids[..])
    .fetch_all(&state.db)
    .await?;

    let mut galleries: HashMap<Uuid, Vec<ProductImage>> = HashMap::new();
    for row in rows {
        galleries.entry(row.product_id).or_default().push(ProductImage {
            id: row.id,
            image_url: row.image_url,
            mime_type: row.mime_type,
            byte_size: row.byte_size,
            sort_order: row.sort_order,
            is_primary: row.is_primary,
        });
    }

    Ok(products
        .into_iter()
        .map(|product| ProductResponse {
            images: galleries.remove(&product.id).unwrap_or_default(),
            product,
        })
        .collect())
}

async fn lock_seller_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: Uuid,
    seller_id: Uuid,
) -> Result<(), AppError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM products WHERE id = $1 AND seller_id = $2 FOR UPDATE",
    )
    .bind(product_id)
    .bind(seller_id)
    .fetch_optional(&mut **tx)
    .await?;
    found.ok_or(AppError::NotFound)?;
    Ok(())
}

async fn get_owned_product(
    state: &AppState,
    product_id: Uuid,
    seller_id: Uuid,
) -> Result<Product, AppError> {
    sqlx::query_as::<_, Product>(
        r#"SELECT id, title, description, price_kes, category_id, image_url,
                  condition, status, created_at
           FROM products WHERE id = $1 AND seller_id = $2"#,
    )
    .bind(product_id)
    .bind(seller_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

async fn normalize_gallery_order(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("UPDATE product_images SET sort_order = sort_order + 8, is_primary = FALSE WHERE product_id = $1")
        .bind(product_id)
        .execute(&mut **tx)
        .await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT uploaded_image_id FROM product_images WHERE product_id = $1 ORDER BY sort_order",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await?;
    for (index, image_id) in ids.iter().enumerate() {
        sqlx::query("UPDATE product_images SET sort_order = $3 WHERE product_id = $1 AND uploaded_image_id = $2")
            .bind(product_id)
            .bind(image_id)
            .bind(index as i16)
            .execute(&mut **tx)
            .await?;
    }
    if let Some(primary_id) = ids.first() {
        sqlx::query("UPDATE product_images SET is_primary = TRUE WHERE product_id = $1 AND uploaded_image_id = $2")
            .bind(product_id)
            .bind(primary_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn sync_primary_image_url(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE products
           SET image_url = (
               SELECT ui.public_url
               FROM product_images pi
               JOIN uploaded_images ui ON ui.id = pi.uploaded_image_id
               WHERE pi.product_id = $1 AND pi.is_primary = TRUE
               LIMIT 1
           ), updated_at = NOW()
           WHERE id = $1"#,
    )
    .bind(product_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
