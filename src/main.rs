mod auth;
mod ai;
mod uploads;
mod models;
mod routes;
mod state;
mod error;

use axum::{extract::DefaultBodyLimit, http::{header, HeaderValue, Method}, routing::{delete, get, post, put}, Router};
use sqlx::postgres::PgPoolOptions;
use std::{env, net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{auth::{routes as auth_routes, sms::build_sms_sender}, routes::{health, products, categories}, state::AppState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "sokorelay_api=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new().max_connections(10).connect(&database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let sms = build_sms_sender()?;
    let state = AppState { db: pool, sms: Arc::from(sms) };

    let origins = env::var("CORS_ORIGINS").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let allowed_origins: Vec<HeaderValue> = origins.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let app = Router::new()
        .route("/health", get(health::health))
        .route("/api/v1/auth/otp/request", post(auth_routes::request_otp))
        .route("/api/v1/auth/otp/verify", post(auth_routes::verify_otp))
        .route("/api/v1/auth/refresh", post(auth_routes::refresh))
        .route("/api/v1/auth/logout", post(auth_routes::logout))
        .route("/api/v1/auth/me", get(auth_routes::me))
        .route("/api/v1/categories", get(categories::list_categories))
        .route("/api/v1/products", get(products::list_products).post(products::create_product))
        .route("/api/v1/products/:id", get(products::get_product))
        .route("/api/v1/products/:id/images", post(products::add_product_images))
        .route("/api/v1/products/:id/images/reorder", put(products::reorder_product_images))
        .route("/api/v1/products/:id/images/:image_id", delete(products::remove_product_image))
        .route("/api/v1/uploads/images", post(uploads::upload_image))
        .route("/api/v1/uploads/images/batch", post(uploads::upload_images_batch))
        .route("/api/v1/ai/listing-suggestions", post(ai::suggest_listing))
        .nest_service("/media", ServeDir::new("uploads"))
        .layer(DefaultBodyLimit::max(33 * 1024 * 1024))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT").unwrap_or_else(|_| "8080".to_string()).parse()?;
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    tracing::info!("SokoRelay API listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
