//! ntfy failure notifications for scheduled sync runs.

use std::time::Duration;

use crate::config::NtfyConfig;

/// Publish a short, non-sensitive failure summary to the configured topic.
pub fn notify(config: &NtfyConfig, message: &str) -> Result<(), String> {
    let server = config.server.trim().trim_end_matches('/');
    if server.is_empty() {
        return Err("ntfy server URL is empty".to_string());
    }
    let topic = config.topic.trim();
    if topic.is_empty() {
        return Err("ntfy topic is empty".to_string());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "could not create HTTP client".to_string())?;
    let mut request = client
        .post(format!("{server}/{topic}"))
        .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.to_string());
    if let Some(token) = config.token.as_deref().filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(token);
    }

    let response = request.send().map_err(|_| "request failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!("server returned HTTP {}", response.status()));
    }
    Ok(())
}
