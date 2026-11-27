use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Serialize;
use tokio::fs;
use uuid::Uuid;

use crate::{auth::routes::authenticated_user_id, error::AppError, state::AppState};

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_BATCH_IMAGES: usize = 8;
const MAX_BATCH_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct UploadedImageResponse {
    pub id: Uuid,
    pub image_url: String,
    pub mime_type: String,
    pub byte_size: usize,
}

#[derive(Debug, Serialize)]
pub struct BatchUploadResponse {
    pub images: Vec<UploadedImageResponse>,
}

struct PendingImage {
    bytes: bytes::Bytes,
    mime_type: &'static str,
    extension: &'static str,
}

/// Backwards-compatible single-image upload using one multipart `file` field.
pub async fn upload_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadedImageResponse>), AppError> {
    let owner_id = authenticated_user_id(&state, &headers).await?;
    ensure_local_storage_enabled()?;

    let mut pending: Option<PendingImage> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("Invalid multipart upload".into()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        if pending.is_some() {
            return Err(AppError::BadRequest("Upload one image per request to this endpoint".into()));
        }
        pending = Some(read_image_field(field).await?);
    }

    let pending = pending.ok_or_else(|| {
        AppError::BadRequest("Attach an image in a multipart field named 'file'".into())
    })?;
    let saved = persist_image(&state, owner_id, pending).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

/// Batch upload accepts 1-8 repeated multipart fields, all named `file`.
/// Use this when a seller wants to add several gallery views in one request.
pub async fn upload_images_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<BatchUploadResponse>), AppError> {
    let owner_id = authenticated_user_id(&state, &headers).await?;
    ensure_local_storage_enabled()?;

    let mut pending = Vec::new();
    let mut total_bytes = 0usize;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("Invalid multipart upload".into()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        if pending.len() >= MAX_BATCH_IMAGES {
            return Err(AppError::BadRequest("A listing can have at most 8 images".into()));
        }
        let image = read_image_field(field).await?;
        total_bytes = total_bytes.saturating_add(image.bytes.len());
        if total_bytes > MAX_BATCH_BYTES {
            return Err(AppError::BadRequest("Combined image upload must be 32 MB or smaller".into()));
        }
        pending.push(image);
    }
    if pending.is_empty() {
        return Err(AppError::BadRequest(
            "Attach one or more images using repeated multipart fields named 'file'".into(),
        ));
    }

    let images = persist_images_batch(&state, owner_id, pending).await?;
    Ok((StatusCode::CREATED, Json(BatchUploadResponse { images })))
}

fn ensure_local_storage_enabled() -> Result<(), AppError> {
    let storage_mode = std::env::var("IMAGE_STORAGE_MODE").unwrap_or_else(|_| "local".to_string());
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
    if storage_mode != "local" || app_env == "production" {
        return Err(AppError::StorageUnavailable);
    }
    Ok(())
}

async fn read_image_field(field: axum::extract::multipart::Field<'_>) -> Result<PendingImage, AppError> {
    let bytes = field
        .bytes()
        .await
        .map_err(|_| AppError::BadRequest("Could not read uploaded image".into()))?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest("The uploaded image is empty".into()));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::BadRequest("Each image must be 8 MB or smaller".into()));
    }
    let (mime_type, extension) = detect_image_type(&bytes)
        .ok_or_else(|| AppError::BadRequest("Only JPEG, PNG and WEBP images are supported".into()))?;
    Ok(PendingImage { bytes, mime_type, extension })
}

async fn persist_image(
    state: &AppState,
    owner_id: Uuid,
    pending: PendingImage,
) -> Result<UploadedImageResponse, AppError> {
    let id = Uuid::new_v4();
    let file_name = format!("{id}.{}", pending.extension);
    fs::create_dir_all("uploads")
        .await
        .map_err(|_| AppError::StorageUnavailable)?;
    let file_path = format!("uploads/{file_name}");
    fs::write(&file_path, &pending.bytes)
        .await
        .map_err(|_| AppError::StorageUnavailable)?;

    let image_url = format!("/media/{file_name}");
    let insert_result = sqlx::query(
        "INSERT INTO uploaded_images (id, owner_id, storage_provider, file_name, public_url, mime_type, byte_size) VALUES ($1, $2, 'local', $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(owner_id)
    .bind(&file_name)
    .bind(&image_url)
    .bind(pending.mime_type)
    .bind(pending.bytes.len() as i64)
    .execute(&state.db)
    .await;

    if let Err(error) = insert_result {
        let _ = fs::remove_file(&file_path).await;
        return Err(error.into());
    }

    Ok(UploadedImageResponse {
        id,
        image_url,
        mime_type: pending.mime_type.to_string(),
        byte_size: pending.bytes.len(),
    })
}


async fn persist_images_batch(
    state: &AppState,
    owner_id: Uuid,
    pending: Vec<PendingImage>,
) -> Result<Vec<UploadedImageResponse>, AppError> {
    fs::create_dir_all("uploads")
        .await
        .map_err(|_| AppError::StorageUnavailable)?;

    let mut file_rows: Vec<(Uuid, String, String, PendingImage)> = Vec::with_capacity(pending.len());
    for image in pending {
        let id = Uuid::new_v4();
        let file_name = format!("{id}.{}", image.extension);
        let file_path = format!("uploads/{file_name}");
        if let Err(error) = fs::write(&file_path, &image.bytes).await {
            for (_, previous_name, _, _) in &file_rows {
                let _ = fs::remove_file(format!("uploads/{previous_name}")).await;
            }
            let _ = fs::remove_file(&file_path).await;
            tracing::error!(error = %error, "Could not write a batched product image");
            return Err(AppError::StorageUnavailable);
        }
        let image_url = format!("/media/{file_name}");

file_rows.push((id, file_name, image_url, image));
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            for (_, file_name, _, _) in &file_rows {
                let _ = fs::remove_file(format!("uploads/{file_name}")).await;
            }
            return Err(error.into());
        }
    };

    for (id, file_name, image_url, image) in &file_rows {
        let insert_result = sqlx::query(
            "INSERT INTO uploaded_images (id, owner_id, storage_provider, file_name, public_url, mime_type, byte_size) VALUES ($1, $2, 'local', $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(owner_id)
        .bind(file_name)
        .bind(image_url)
        .bind(image.mime_type)
        .bind(image.bytes.len() as i64)
        .execute(&mut *tx)
        .await;

        if let Err(error) = insert_result {
            let _ = tx.rollback().await;
            for (_, saved_name, _, _) in &file_rows {
                let _ = fs::remove_file(format!("uploads/{saved_name}")).await;
            }
            return Err(error.into());
        }
    }

    if let Err(error) = tx.commit().await {
        for (_, file_name, _, _) in &file_rows {
            let _ = fs::remove_file(format!("uploads/{file_name}")).await;
        }
        return Err(error.into());
    }

    Ok(file_rows
        .into_iter()
        .map(|(id, _file_name, image_url, image)| UploadedImageResponse {
            id,
            image_url,
            mime_type: image.mime_type.to_string(),
            byte_size: image.bytes.len(),
        })
        .collect())
}

fn detect_image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        Some(("image/png", "png"))
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}
