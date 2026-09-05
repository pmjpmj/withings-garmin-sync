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
const MEASURE_PATH: &str = "/measure";

/// meastype code the CLI reads for weight: 1 = weight (kg).
pub const WEIGHT_MEASTYPES: &str = "1";
/// meastype codes the CLI reads for blood pressure: 9 = diastolic, 10 =
/// systolic, 11 = pulse.
pub const BP_MEASTYPES: &str = "9,10,11";
/// Every meastype code the CLI reads.
pub const ALL_MEASTYPES: &str = "1,9,10,11";
/// Safety cap on pagination, so a misbehaving `more` cannot loop forever.
const MAX_PAGES: usize = 50;

/// One raw measure inside a Withings measure group.
#[derive(Debug, Clone, PartialEq)]
pub struct RawMeasure {
    /// meastype: 1 = weight (kg), 9 = diastolic BP, 10 = systolic BP, 11 = pulse.
    pub meastype: u32,
    pub value: i64,
    pub unit: i32,
}

/// A Withings measure group: measures taken at the same time.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureGroup {
    /// Measurement time, epoch seconds (UTC).
    pub date: i64,
    pub measures: Vec<RawMeasure>,
}

/// Decode a Withings value to its real number: `value * 10^unit`.
pub fn decode_value(value: i64, unit: i32) -> f64 {
    value as f64 * 10f64.powi(unit)
}

/// Withings signals rate-limiting as HTTP status 601 (per their docs).
fn is_rate_limited(status: u16) -> bool {
    status == 601
}

/// Run a Withings POST up to 3 times, retrying 601s with backoff.
fn post_with_601_retry<T>(
    operation: impl FnMut() -> Result<(u16, T), AppError>,
) -> Result<(u16, T), AppError> {
    crate::http::retry(3, |(status, _)| is_rate_limited(*status), operation)
}

/// Read the measurement window from Withings (`action=getmeas`), following
/// the response's `more`/`offset` fields until there are no more pages.
/// `meastypes` is the comma-separated set of meastype codes to request
/// (ADR-0005 scopes it per metric). Withings `601` rate-limit responses are
/// retried with backoff.
pub fn read_measures(
    client: &HttpClient,
    access_token: &str,
    startdate: i64,
    enddate: i64,
    meastypes: &str,
) -> Result<Vec<MeasureGroup>, AppError> {
    let url = client.withings_url(MEASURE_PATH);
    let auth = format!("Bearer {access_token}");

    let mut groups: Vec<MeasureGroup> = Vec::new();
    let mut offset: u64 = 0;
    for _page in 0..MAX_PAGES {
        let fields = vec![
            ("action".to_string(), "getmeas".to_string()),
            ("meastypes".to_string(), meastypes.to_string()),
            ("startdate".to_string(), startdate.to_string()),
            ("enddate".to_string(), enddate.to_string()),
            ("offset".to_string(), offset.to_string()),
        ];

        // Withings `601` (rate-limited) is retried with backoff, then the
        // response is parsed like any other page.
        let (status, body) = post_with_601_retry(|| {
            client
                .post_form_headers(&url, &fields, &[("Authorization", auth.as_str())], None)
                .map_err(|error| {
                    AppError::new(
                        crate::EXIT_METRIC_FAILURE,
                        format!("could not read measurements from Withings: {error}"),
                    )
                })
        })?;
        if !(200..300).contains(&status) {
            return Err(AppError::new(
                crate::EXIT_METRIC_FAILURE,
                format!("Withings measurements request returned HTTP {status}"),
            ));
        }
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            AppError::new(
                crate::EXIT_METRIC_FAILURE,
                format!("Withings measurements response was invalid JSON: {error}"),
            )
        })?;
        let withings_status = json.get("status").and_then(|v| v.as_i64()).unwrap_or(-1);
        if withings_status != 0 {
            return Err(AppError::new(
                crate::EXIT_METRIC_FAILURE,
                format!("Withings rejected the measurements request (status {withings_status})"),
            ));
        }

        let body = json.get("body").cloned().unwrap_or(serde_json::Value::Null);
        if let Some(page) = body.get("measuregrps").and_then(|v| v.as_array()) {
            for group in page {
                let Some(date) = group.get("date").and_then(|v| v.as_i64()) else {
                    continue;
                };
                let measures = group
                    .get("measures")
                    .and_then(|v| v.as_array())
                    .map(|measures| {
                        measures
                            .iter()
                            .filter_map(|m| {
                                Some(RawMeasure {
                                    meastype: m.get("type")?.as_u64()? as u32,
                                    value: m.get("value")?.as_i64()?,
                                    unit: m.get("unit")?.as_i64()? as i32,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                groups.push(MeasureGroup { date, measures });
            }
        }

        let more = body.get("more").and_then(|v| v.as_u64()).unwrap_or(0);
        if more == 0 {
            return Ok(groups);
        }
        let next = body.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
        if next == offset {
            // The server says "more" but gave no new offset; stop rather
            // than loop.
            return Ok(groups);
        }
        offset = next;
    }

    Err(AppError::new(
        crate::EXIT_METRIC_FAILURE,
        format!("Withings pagination did not terminate after {MAX_PAGES} pages"),
    ))
}

/// Generate a fresh random `state` for one authorization round-trip.
/// Withings requires the parameter on the authorize URL; the CLI validates
/// the value Withings echoes back when the operator pastes the redirect URL.
pub fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the operating system refused to provide randomness");
    let mut state = String::with_capacity(32);
    for byte in bytes {
        state.push_str(&format!("{byte:02x}"));
    }
    state
}

/// Build the Withings authorize URL for the operator to open in a browser.
/// Withings requires a `state` query parameter (their API marks it
/// `required: true`); the caller generates it with [`generate_state`].
pub fn authorize_url(client_id: &str, state: &str) -> String {
    format!(
        "{}/oauth2_user/authorize2?response_type=code&client_id={}&scope={}&redirect_uri={}&state={}",
        AUTHORIZE_HOST,
        crate::http::query_encode(client_id),
        crate::http::query_encode("user.metrics"),
        crate::http::query_encode(REDIRECT_URI),
        crate::http::query_encode(state),
    )
}

/// Extract the OAuth `code` from whatever the operator pasted: either a full
/// redirect URL (`...?code=XYZ&state=...`) or the bare code. When the pasted
/// string carries a `state`, it must match `state` (this run's generated
/// value); a mismatch is rejected. A bare code has no state to compare and
/// is accepted as-is.
pub fn extract_code(pasted: &str, state: &str) -> Result<String, AppError> {
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            "no authorization code provided; open the URL above, authorize, and paste the redirect",
        ));
    }
    if let Some(marker) = pasted.find("state=") {
        let rest = &pasted[marker + "state=".len()..];
        let end = rest.find(['&', '#']).unwrap_or(rest.len());
        let echoed = rest[..end].trim();
        if echoed != state {
            return Err(AppError::new(
                crate::EXIT_AUTH,
                "the pasted redirect's state does not match this run; re-run `auth`",
            ));
        }
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

/// Exchange an authorization code for Withings tokens. `601` rate-limit
/// responses are retried with backoff.
pub fn exchange_code(
    client: &HttpClient,
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<TokenResponse, AppError> {
    let url = client.withings_url(TOKEN_PATH);
    let fields = exchange_form(client_id, client_secret, code, REDIRECT_URI);
    let (status, body) =
        post_with_601_retry(|| client.post_form(&url, &fields).map_err(map_transport_error))?;
    parse_token_response(status, &body)
}

/// Refresh Withings tokens with a stored refresh token. `601` rate-limit
/// responses are retried with backoff.
pub fn refresh(
    client: &HttpClient,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, AppError> {
    let url = client.withings_url(TOKEN_PATH);
    let fields = refresh_form(client_id, client_secret, refresh_token);
    let (status, body) =
        post_with_601_retry(|| client.post_form(&url, &fields).map_err(map_transport_error))?;
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
    fn authorize_url_encodes_client_id_scope_redirect_and_state() {
        let url = authorize_url("cli&id", "state&more");
        assert!(url.starts_with("https://account.withings.com/oauth2_user/authorize2?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cli%26id"));
        assert!(url.contains("scope=user.metrics"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8765%2F"));
        assert!(url.contains("state=state%26more"));
    }

    #[test]
    fn generate_state_is_32_hex_characters() {
        let state = generate_state();
        assert_eq!(state.len(), 32);
        assert!(state.chars().all(|c| c.is_ascii_hexdigit()), "{state}");
    }

    #[test]
    fn extract_code_handles_full_redirect_url() {
        let code = extract_code("http://localhost:8765/?code=abc123&state=xyz", "xyz").unwrap();
        assert_eq!(code, "abc123");
        // state may precede code in the query string
        let code = extract_code("http://localhost:8765/?state=xyz&code=abc123", "xyz").unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn extract_code_accepts_url_without_state() {
        let code = extract_code("http://localhost:8765/?code=abc123", "irrelevant").unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn extract_code_rejects_mismatched_state() {
        let error =
            extract_code("http://localhost:8765/?code=abc123&state=nope", "xyz").unwrap_err();
        assert_eq!(error.code, crate::EXIT_AUTH);
        assert!(error.message.contains("state"), "{}", error.message);
    }

    #[test]
    fn extract_code_handles_bare_code() {
        assert_eq!(extract_code("abc123", "irrelevant").unwrap(), "abc123");
        assert_eq!(extract_code("  abc123\n", "irrelevant").unwrap(), "abc123");
    }

    #[test]
    fn extract_code_rejects_empty_and_codeless_url() {
        assert!(extract_code("", "xyz").is_err());
        assert!(extract_code("http://localhost:8765/?state=xyz", "xyz").is_err());
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
    fn decode_value_applies_power_of_ten() {
        assert_eq!(decode_value(82400, -3), 82.4);
        assert_eq!(decode_value(120, 0), 120.0);
        assert_eq!(decode_value(5, 2), 500.0);
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
