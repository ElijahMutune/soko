use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error")]
    Database(#[from] sqlx::Error),
    #[error("Invalid input: {0}")]
    BadRequest(String),
    #[error("Resource not found")]
    NotFound,
    #[error("Invalid or expired credentials")]
    InvalidCredentials,
    #[error("Too many requests")]
    RateLimited,
    #[error("Forbidden")]
    Forbidden,
    #[error("Service unavailable")]
    ServiceUnavailable,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Database(err) => {
                tracing::error!(error = %err, "database operation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
            }
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found".to_string()),
            AppError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "Invalid or expired credentials".to_string()),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "Too many requests. Try again later.".to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "This account cannot sign in".to_string()),
            AppError::ServiceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "Authentication service is not configured".to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
