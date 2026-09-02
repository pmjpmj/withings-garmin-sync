//! Withings OAuth2 protocol and token endpoint (ticket 05).
//!
//! The CLI never drives a browser: `auth` prints the authorize URL, the
//! operator opens it, and pastes the redirect URL (or bare code) back. The
//! exchange and refresh both happen at `POST /v2/oauth2` with
//! `action=requesttoken`; `grant_type` selects the operation.

use crate::http::{HttpClient, HttpError};
use crate::AppError;

/// Host of the Withings OAuth authorize page. Not covered by the
/// `WGS_*_BASE` seam overrides because the CLI only *prints* this URL — it
/// never requests it.
pub const AUTHORIZE_HOST: &str = "https://account.withings.com";
/// Redirect URI the CLI registers for the authorization code. The operator
/// must register the same value in the Withings developer portal.
pub const REDIRECT_URI: &str = "http://localhost:8765/";

const TOKEN_PATH: &str = "/v2/oauth2";

/// Build the Withings authorize URL for the operator to open in a browser.
pub fn authorize_url(client_id: &str) -> String {
    format!(
        "{}/oauth2_user/authorize2?response_type=code&client_id={}&scope={}&redirect_uri={}",
        AUTHORIZE_HOST,
        crate::http::query_encode(client_id),
        crate::http::query_encode("user.metrics"),
        crate::http::query_encode(REDIRECT_URI),
    )
}

/// Extract the OAuth `code` from whatever the operator pasted: either a full
/// redirect URL (`...?code=XYZ&state=...`) or the bare code.
pub fn extract_code(pasted: &str) -> Result<String, AppError> {
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            "no authorization code provided; open the URL above, authorize, and paste the redirect",
        ));
    }
    let Some(marker) = pasted.find("code=") else {
        // A pasted URL without a `code` parameter is an error; a bare code is
        // passed through as-is.
        if pasted.starts_with("http://") || pasted.starts_with("https://") {
            return Err(AppError::new(
                crate::EXIT_AUTH,
                "the pasted redirect URL contains no code",
            ));
        }
        return Ok(pasted.to_string());
    };
    let rest = &pasted[marker + "code=".len()..];
    let end = rest.find(['&', '#']).unwrap_or(rest.len());
    let code = rest[..end].trim();
    if code.is_empty() {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            "the pasted redirect URL contains no code",
        ));
    }
    Ok(code.to_string())
}

/// Exact form fields for the authorization-code exchange.
pub fn exchange_form(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Vec<(String, String)> {
    vec![
        ("action".into(), "requesttoken".into()),
        ("grant_type".into(), "authorization_code".into()),
        ("client_id".into(), client_id.into()),
        ("client_secret".into(), client_secret.into()),
        ("code".into(), code.into()),
        ("redirect_uri".into(), redirect_uri.into()),
    ]
}

/// Exact form fields for a refresh-token grant.
pub fn refresh_form(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Vec<(String, String)> {
    vec![
        ("action".into(), "requesttoken".into()),
        ("grant_type".into(), "refresh_token".into()),
        ("client_id".into(), client_id.into()),
        ("client_secret".into(), client_secret.into()),
        ("refresh_token".into(), refresh_token.into()),
    ]
}

/// A successful token response (`status == 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    /// Present on exchange; on refresh Withings may rotate it, and the caller
    /// replaces the stored token only when this is `Some`.
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

/// Exchange an authorization code for Withings tokens.
pub fn exchange_code(
    client: &HttpClient,
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<TokenResponse, AppError> {
    let url = client.withings_url(TOKEN_PATH);
    let fields = exchange_form(client_id, client_secret, code, REDIRECT_URI);
    let (status, body) = client
        .post_form(&url, &fields)
        .map_err(map_transport_error)?;
    parse_token_response(status, &body)
}

/// Refresh Withings tokens with a stored refresh token.
pub fn refresh(
    client: &HttpClient,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, AppError> {
    let url = client.withings_url(TOKEN_PATH);
    let fields = refresh_form(client_id, client_secret, refresh_token);
    let (status, body) = client
        .post_form(&url, &fields)
        .map_err(map_transport_error)?;
    parse_token_response(status, &body)
}

fn map_transport_error(error: HttpError) -> AppError {
    AppError::new(
        crate::EXIT_AUTH,
        format!("could not reach the Withings token endpoint: {error}"),
    )
}

fn parse_token_response(status: u16, body: &str) -> Result<TokenResponse, AppError> {
    if !(200..300).contains(&status) {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Withings token endpoint returned HTTP {status}"),
        ));
    }
    let json: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        AppError::new(
            crate::EXIT_AUTH,
            format!("Withings token endpoint returned invalid JSON: {error}"),
        )
    })?;

    let withings_status = json.get("status").and_then(|v| v.as_i64()).unwrap_or(-1);
    if withings_status != 0 {
        return Err(withings_error(withings_status));
    }

    let body = json.get("body").ok_or_else(|| {
        AppError::new(
            crate::EXIT_AUTH,
            "Withings token response was missing its body",
        )
    })?;
    let access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::new(
                crate::EXIT_AUTH,
                "Withings token response had no access_token",
            )
        })?
        .to_string();
    let refresh_token = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            AppError::new(
                crate::EXIT_AUTH,
                "Withings token response had no expires_in",
            )
        })?;

    Ok(TokenResponse {
        access_token,
        refresh_token,
        expires_in,
    })
}

fn withings_error(status: i64) -> AppError {
    let message = match status {
        2556 => "the authorization code was rejected (status 2556: invalid or expired)".to_string(),
        601 => "Withings is rate-limiting token requests (status 601); try again in a minute"
            .to_string(),
        other => format!("the Withings token request failed (status {other})"),
    };
    AppError::new(
        crate::EXIT_AUTH,
        format!("Withings rejected the request: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_encodes_client_id_scope_and_redirect() {
        let url = authorize_url("cli&id");
        assert!(url.starts_with("https://account.withings.com/oauth2_user/authorize2?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cli%26id"));
        assert!(url.contains("scope=user.metrics"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8765%2F"));
    }

    #[test]
    fn extract_code_handles_full_redirect_url() {
        let code = extract_code("http://localhost:8765/?code=abc123&state=xyz").unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn extract_code_handles_bare_code() {
        assert_eq!(extract_code("abc123").unwrap(), "abc123");
        assert_eq!(extract_code("  abc123\n").unwrap(), "abc123");
    }

    #[test]
    fn extract_code_rejects_empty_and_codeless_url() {
        assert!(extract_code("").is_err());
        assert!(extract_code("http://localhost:8765/?state=xyz").is_err());
    }

    #[test]
    fn exchange_form_has_exact_withings_fields() {
        let fields = exchange_form("cid", "secret", "code", "http://localhost:8765/");
        assert_eq!(
            fields,
            vec![
                ("action".into(), "requesttoken".into()),
                ("grant_type".into(), "authorization_code".into()),
                ("client_id".into(), "cid".into()),
                ("client_secret".into(), "secret".into()),
                ("code".into(), "code".into()),
                ("redirect_uri".into(), "http://localhost:8765/".into()),
            ]
        );
    }

    #[test]
    fn refresh_form_has_exact_withings_fields() {
        let fields = refresh_form("cid", "secret", "old-refresh");
        assert_eq!(
            fields,
            vec![
                ("action".into(), "requesttoken".into()),
                ("grant_type".into(), "refresh_token".into()),
                ("client_id".into(), "cid".into()),
                ("client_secret".into(), "secret".into()),
                ("refresh_token".into(), "old-refresh".into()),
            ]
        );
    }

    #[test]
    fn parses_successful_token_response() {
        let parsed = parse_token_response(
            200,
            r#"{"status":0,"body":{"userid":42,"access_token":"at","refresh_token":"rt","expires_in":10800,"scope":"user.metrics","token_type":"Bearer"}}"#,
        )
        .unwrap();
        assert_eq!(
            parsed,
            TokenResponse {
                access_token: "at".into(),
                refresh_token: Some("rt".into()),
                expires_in: 10800,
            }
        );
    }

    #[test]
    fn parses_error_status_with_clear_message() {
        let error = parse_token_response(200, r#"{"status":2556}"#).unwrap_err();
        assert_eq!(error.code, crate::EXIT_AUTH);
        assert!(error.message.contains("2556"), "{}", error.message);
        assert!(error.message.contains("rejected"), "{}", error.message);
    }

    #[test]
    fn rejects_non_2xx_and_invalid_json() {
        let error = parse_token_response(500, "boom").unwrap_err();
        assert!(error.message.contains("500"), "{}", error.message);

        let error = parse_token_response(200, "not json").unwrap_err();
        assert!(error.message.contains("invalid JSON"), "{}", error.message);
    }
}
