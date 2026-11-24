use std::env;

/// Provider boundary for SMS delivery. Implement this trait for Africa's Talking,
/// Twilio, or another provider without changing the authentication flow.
#[async_trait::async_trait]
pub trait SmsSender: Send + Sync {
    async fn send_otp(&self, phone_e164: &str, code: &str) -> Result<(), String>;
}

/// Development-only sender. The code is written to server logs, never returned
/// in the HTTP response. Do not enable this mode in production.
pub struct ConsoleSmsSender;

#[async_trait::async_trait]
impl SmsSender for ConsoleSmsSender {
    async fn send_otp(&self, phone_e164: &str, code: &str) -> Result<(), String> {
        tracing::warn!(phone = %phone_e164, otp = %code, "DEV ONLY: simulated SMS OTP");
        Ok(())
    }
}

pub fn build_sms_sender() -> Result<Box<dyn SmsSender>, Box<dyn std::error::Error>> {
    let mode = env::var("SMS_MODE").unwrap_or_else(|_| "console".to_string());
    let environment = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
    match mode.as_str() {
        "console" if environment != "production" => Ok(Box::new(ConsoleSmsSender)),
        "console" => Err("SMS_MODE=console is forbidden when APP_ENV=production".into()),
        other => Err(format!("Unsupported SMS_MODE '{other}'. Implement/configure a provider adapter before enabling it.").into()),
    }
}
