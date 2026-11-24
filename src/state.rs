use std::sync::Arc;
use sqlx::PgPool;
use crate::auth::sms::SmsSender;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub sms: Arc<dyn SmsSender>,
}
