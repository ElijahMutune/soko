use axum::{extract::State, http::{HeaderMap, StatusCode}, Json};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

const OTP_TTL_MINUTES: i64 = 5;
const OTP_MAX_ATTEMPTS: i32 = 5;
const OTP_MAX_SENDS_PER_HOUR: i64 = 3;
const ACCESS_TTL_MINUTES: i64 = 15;
const REFRESH_TTL_DAYS: i64 = 30;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Deserialize)]
pub struct RequestOtp {
    pub phone: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyOtp {
    pub phone: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub refresh_token: String,
    pub user: UserResponse,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub phone: String,
    pub username: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccessClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: usize,
    exp: usize,
}

pub async fn request_otp(
    State(state): State<AppState>,
    Json(input): Json<RequestOtp>,
) -> Result<(StatusCode, Json<MessageResponse>), AppError> {
    let phone = normalize_kenyan_phone(&input.phone)?;

    let recent_sends: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM otp_challenges WHERE phone_e164 = $1 AND created_at > NOW() - INTERVAL '1 hour'",
    )
    .bind(&phone)
    .fetch_one(&state.db)
    .await?;
    if recent_sends >= OTP_MAX_SENDS_PER_HOUR {
        return Err(AppError::RateLimited);
    }

    let mut random = [0u8; 4];
    OsRng.fill_bytes(&mut random);
    let number = u32::from_be_bytes(random) % 1_000_000;
    let code = format!("{number:06}");
    let digest = otp_digest(&phone, &code)?;

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE otp_challenges SET consumed_at = NOW() WHERE phone_e164 = $1 AND consumed_at IS NULL")
        .bind(&phone)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO otp_challenges (phone_e164, code_digest, expires_at) VALUES ($1, $2, NOW() + ($3 * INTERVAL '1 minute'))",
    )
    .bind(&phone)
    .bind(digest)
    .bind(OTP_TTL_MINUTES as f64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if let Err(error) = state.sms.send_otp(&phone, &code).await {
        tracing::error!(error = %error, "SMS delivery failed");
        return Err(AppError::ServiceUnavailable);
    }

    Ok((StatusCode::ACCEPTED, Json(MessageResponse {
        message: "If this number can receive SMS, a verification code has been sent.",
    })))
}

pub async fn verify_otp(
    State(state): State<AppState>,
    Json(input): Json<VerifyOtp>,
) -> Result<Json<AuthResponse>, AppError> {
    let phone = normalize_kenyan_phone(&input.phone)?;
    if input.code.len() != 6 || !input.code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::BadRequest("Enter the 6-digit verification code".into()));
    }

    let mut tx = state.db.begin().await?;
    let challenge = sqlx::query(
        "SELECT id, code_digest, attempts FROM otp_challenges WHERE phone_e164 = $1 AND consumed_at IS NULL AND expires_at > NOW() ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(&phone)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::InvalidCredentials)?;

    let challenge_id: Uuid = challenge.try_get("id")?;
    let stored_digest: Vec<u8> = challenge.try_get("code_digest")?;
    let attempts: i32 = challenge.try_get("attempts")?;
    if attempts >= OTP_MAX_ATTEMPTS {
        return Err(AppError::RateLimited);
    }

    let candidate = otp_digest(&phone, &input.code)?;
    if !constant_time_eq(&stored_digest, &candidate) {
        sqlx::query("UPDATE otp_challenges SET attempts = attempts + 1 WHERE id = $1")
            .bind(challenge_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::InvalidCredentials);
    }

    sqlx::query("UPDATE otp_challenges SET consumed_at = NOW() WHERE id = $1")
        .bind(challenge_id)
        .execute(&mut *tx)
        .await?;

    let user = sqlx::query(
        "INSERT INTO users (phone_e164, phone_verified_at, account_status, public_username) VALUES ($1, NOW(), 'active', $2) ON CONFLICT (phone_e164) DO UPDATE SET phone_verified_at = COALESCE(users.phone_verified_at, NOW()), account_status = CASE WHEN users.account_status = 'pending' THEN 'active' ELSE users.account_status END RETURNING id, phone_e164, public_username, account_status",
    )
    .bind(&phone)
    .bind(format!("seller_{}", Uuid::new_v4().simple().to_string()[..8].to_string()))
    .fetch_one(&mut *tx)
    .await?;

    let user_id: Uuid = user.try_get("id")?;
    let account_status: String = user.try_get("account_status")?;
    if account_status != "active" {
        tx.commit().await?;
        return Err(AppError::Forbidden);
    }

    let refresh_token = random_token();
    let refresh_hash = token_hash(&refresh_token);
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO auth_sessions (id, user_id, refresh_token_hash, expires_at) VALUES ($1, $2, $3, NOW() + ($4 * INTERVAL '1 day'))",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(refresh_hash)
    .bind(REFRESH_TTL_DAYS as f64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let access_token = create_access_token(user_id)?;
    Ok(Json(AuthResponse {
        access_token,
        token_type: "Bearer",
        expires_in: ACCESS_TTL_MINUTES * 60,
        refresh_token,
        user: UserResponse {
            id: user_id,
            phone,
            username: user.try_get("public_username")?,
        },
    }))
}

pub async fn refresh(
    State(state): State<AppState>,
    Json(input): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    if input.refresh_token.len() < 32 || input.refresh_token.len() > 256 {
        return Err(AppError::InvalidCredentials);
    }
    let old_hash = token_hash(&input.refresh_token);
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        "SELECT s.id AS session_id, s.user_id, u.phone_e164, u.public_username, u.account_status FROM auth_sessions s JOIN users u ON u.id = s.user_id WHERE s.refresh_token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > NOW() FOR UPDATE OF s",
    )
    .bind(old_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::InvalidCredentials)?;

    if row.try_get::<String, _>("account_status")? != "active" {
        return Err(AppError::Forbidden);
    }
    let session_id: Uuid = row.try_get("session_id")?;
    let user_id: Uuid = row.try_get("user_id")?;
    let phone: String = row.try_get("phone_e164")?;
    let username: String = row.try_get("public_username")?;
    let new_refresh = random_token();
    sqlx::query("UPDATE auth_sessions SET refresh_token_hash = $1, expires_at = NOW() + ($2 * INTERVAL '1 day'), last_used_at = NOW() WHERE id = $3")
        .bind(token_hash(&new_refresh))
        .bind(REFRESH_TTL_DAYS as f64)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(AuthResponse {
        access_token: create_access_token(user_id)?,
        token_type: "Bearer",
        expires_in: ACCESS_TTL_MINUTES * 60,
        refresh_token: new_refresh,
        user: UserResponse { id: user_id, phone, username },
    }))
}

pub async fn logout(
    State(state): State<AppState>,
    Json(input): Json<LogoutRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    if input.refresh_token.len() < 32 || input.refresh_token.len() > 256 {
        return Err(AppError::InvalidCredentials);
    }
    sqlx::query("UPDATE auth_sessions SET revoked_at = NOW() WHERE refresh_token_hash = $1 AND revoked_at IS NULL")
        .bind(token_hash(&input.refresh_token))
        .execute(&state.db)
        .await?;
    Ok(Json(MessageResponse { message: "Signed out" }))
}

pub fn verify_access_token(token: &str) -> Result<Uuid, AppError> {
    let secret = std::env::var("JWT_SECRET").map_err(|_| AppError::ServiceUnavailable)?;
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_issuer(&["sokorelay-api"]);
    validation.set_audience(&["sokorelay-client"]);
    let claims = decode::<AccessClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation)
        .map_err(|_| AppError::InvalidCredentials)?
        .claims;
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::InvalidCredentials)
}

fn create_access_token(user_id: Uuid) -> Result<String, AppError> {
    let secret = std::env::var("JWT_SECRET").map_err(|_| AppError::ServiceUnavailable)?;
    if secret.len() < 32 {
        return Err(AppError::ServiceUnavailable);
    }
    let now = Utc::now().timestamp() as usize;
    let claims = AccessClaims {
        sub: user_id.to_string(),
        iss: "sokorelay-api".into(),
        aud: "sokorelay-client".into(),
        iat: now,
        exp: (Utc::now() + Duration::minutes(ACCESS_TTL_MINUTES)).timestamp() as usize,
    };
    encode(&Header::new(jsonwebtoken::Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|_| AppError::ServiceUnavailable)
}

fn normalize_kenyan_phone(raw: &str) -> Result<String, AppError> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let normalized = if digits.starts_with("254") {
        format!("+{digits}")
    } else if digits.starts_with('0') && digits.len() == 10 {
        format!("+254{}", &digits[1..])
    } else if digits.len() == 9 {
        format!("+254{digits}")
    } else {
        return Err(AppError::BadRequest("Use a Kenyan mobile number such as +254712345678".into()));
    };
    if normalized.len() != 13 || !normalized.starts_with("+254") || !matches!(normalized.as_bytes()[4], b'1' | b'7') {
        return Err(AppError::BadRequest("Use a valid Kenyan mobile number such as +254712345678".into()));
    }
    Ok(normalized)
}

fn otp_digest(phone: &str, code: &str) -> Result<Vec<u8>, AppError> {
    let pepper = std::env::var("OTP_PEPPER").map_err(|_| AppError::ServiceUnavailable)?;
    if pepper.len() < 32 {
        return Err(AppError::ServiceUnavailable);
    }
    let mut mac = HmacSha256::new_from_slice(pepper.as_bytes()).map_err(|_| AppError::ServiceUnavailable)?;
    mac.update(phone.as_bytes());
    mac.update(b":");
    mac.update(code.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserResponse>, AppError> {
    let bearer = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(AppError::InvalidCredentials)?;
    let user_id = verify_access_token(bearer)?;
    let row = sqlx::query("SELECT id, phone_e164, public_username, account_status FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::InvalidCredentials)?;
    if row.try_get::<String, _>("account_status")? != "active" {
        return Err(AppError::Forbidden);
    }
    Ok(Json(UserResponse {
        id: row.try_get("id")?,
        phone: row.try_get("phone_e164")?,
        username: row.try_get("public_username")?,
    }))
}
