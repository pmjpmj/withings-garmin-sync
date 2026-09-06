//! Garmin mobile-SSO → DI Bearer authentication (ticket 06).
//!
//! The ordered handshake (traced in ticket 03, proven live in ticket 01):
//!
//! 1. `GET {sso}/mobile/sso/en_US/sign-in?clientId=GCM_ANDROID_DARK&service=…`
//!    establishes SSO session cookies (the shared client's cookie jar keeps
//!    them for the next request).
//! 2. `POST {sso}/mobile/api/login` with the credentials JSON. The response
//!    either carries the CAS `serviceTicketId` (`SUCCESSFUL`), demands an
//!    interactive MFA code (`MFA_REQUIRED`), or rejects the credentials
//!    (`INVALID_USERNAME_PASSWORD`).
//! 3. MFA (if required): `POST {sso}/mobile/api/mfa/verifyCode` with the
//!    operator's code; invalid codes loop back to the prompt.
//! 4. `POST {diauth}/di-oauth2-service/oauth/token` exchanges the CAS ticket
//!    for a DI `access_token` + `refresh_token`, trying the candidate client
//!    ids in order until one works.
//!
//! Refresh uses the same token endpoint with `grant_type=refresh_token`.

use crate::http::{query_encode, HttpClient, HttpError};
use crate::AppError;

pub const MOBILE_SSO_CLIENT_ID: &str = "GCM_ANDROID_DARK";
pub const MOBILE_SSO_SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/android";
/// Android WebView user agent for the SSO endpoints.
pub const MOBILE_SSO_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 13; sdk_gphone64_arm64 Build/TE1A.220922.025; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/132.0.0.0 Mobile Safari/537.36";
/// Candidate DI client ids, tried in order; the winner is persisted.
pub const DI_CLIENT_IDS: [&str; 3] = [
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
];
pub const DI_GRANT_TYPE: &str =
    "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";

const SSO_SIGN_IN_PATH: &str = "/mobile/sso/en_US/sign-in";
const SSO_LOGIN_PATH: &str = "/mobile/api/login";
const SSO_MFA_PATH: &str = "/mobile/api/mfa/verifyCode";
const DI_TOKEN_PATH: &str = "/di-oauth2-service/oauth/token";

/// The native Android header set Garmin's API expects on native (DI) calls.
pub fn native_headers() -> [(&'static str, &'static str); 9] {
    [
        ("User-Agent", "GCM-Android-5.23"),
        (
            "X-Garmin-User-Agent",
            "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0",
        ),
        ("X-Garmin-Paired-App-Version", "10861"),
        ("X-Garmin-Client-Platform", "Android"),
        ("X-App-Ver", "10861"),
        ("X-Lang", "en"),
        ("X-GCExperience", "GC5"),
        ("Accept", "application/json"),
        ("Cache-Control", "no-cache"),
    ]
}

/// Headers for the SSO web endpoints (WebView UA, HTML accept).
fn sso_headers() -> [(&'static str, &'static str); 3] {
    [
        ("User-Agent", MOBILE_SSO_USER_AGENT),
        (
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        ),
        ("Accept-Language", "en-US,en;q=0.9"),
    ]
}

fn sign_in_url(sso_base: &str) -> String {
    format!(
        "{sso_base}{SSO_SIGN_IN_PATH}?clientId={}&service={}",
        query_encode(MOBILE_SSO_CLIENT_ID),
        query_encode(MOBILE_SSO_SERVICE_URL),
    )
}

fn login_url(sso_base: &str) -> String {
    format!(
        "{sso_base}{SSO_LOGIN_PATH}?clientId={}&locale=en-US&service={}",
        query_encode(MOBILE_SSO_CLIENT_ID),
        query_encode(MOBILE_SSO_SERVICE_URL),
    )
}

fn mfa_url(sso_base: &str) -> String {
    format!(
        "{sso_base}{SSO_MFA_PATH}?clientId={}&locale=en-US&service={}",
        query_encode(MOBILE_SSO_CLIENT_ID),
        query_encode(MOBILE_SSO_SERVICE_URL),
    )
}

pub fn login_json(username: &str, password: &str) -> serde_json::Value {
    serde_json::json!({
        "username": username,
        "password": password,
        "rememberMe": true,
        "captchaToken": ""
    })
}

pub fn verify_json(method: &str, code: &str) -> serde_json::Value {
    serde_json::json!({
        "mfaMethod": method,
        "mfaVerificationCode": code,
        "rememberMyBrowser": true,
        "reconsentList": [],
        "mfaSetup": false
    })
}

pub fn token_form(client_id: &str, ticket: &str) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), client_id.into()),
        ("service_ticket".into(), ticket.into()),
        ("grant_type".into(), DI_GRANT_TYPE.into()),
        ("service_url".into(), MOBILE_SSO_SERVICE_URL.into()),
    ]
}

pub fn refresh_form(client_id: &str, refresh_token: &str) -> Vec<(String, String)> {
    vec![
        ("grant_type".into(), "refresh_token".into()),
        ("client_id".into(), client_id.into()),
        ("refresh_token".into(), refresh_token.into()),
    ]
}

pub fn basic_auth_header(client_id: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:"));
    format!("Basic {encoded}")
}

// --------------------------------------------------------------------------
// Write endpoints (ticket 07)
// --------------------------------------------------------------------------

const WEIGHT_PATH: &str = "/weight-service/user-weight";
const BP_PATH: &str = "/bloodpressure-service/bloodpressure";

/// Build the weight write payload (shape confirmed live in ticket 01).
pub fn weight_payload(local: &str, gmt: &str, kg: f64) -> serde_json::Value {
    serde_json::json!({
        "dateTimestamp": local,
        "gmtTimestamp": gmt,
        "unitKey": "kg",
        "sourceType": "MANUAL",
        "value": kg
    })
}

/// Build the blood-pressure write payload. `pulse` is omitted when absent;
/// `notes` is only present when non-empty (always empty here). Whole numbers
/// go on the wire as JSON integers, matching the spike's observed payloads.
pub fn bp_payload(
    local: &str,
    gmt: &str,
    systolic: f64,
    diastolic: f64,
    pulse: Option<f64>,
) -> serde_json::Value {
    fn num(value: f64) -> serde_json::Value {
        if value.fract() == 0.0 {
            serde_json::json!(value as i64)
        } else {
            serde_json::json!(value)
        }
    }
    let mut payload = serde_json::json!({
        "measurementTimestampLocal": local,
        "measurementTimestampGMT": gmt,
        "systolic": num(systolic),
        "diastolic": num(diastolic),
        "sourceType": "MANUAL"
    });
    if let Some(pulse) = pulse {
        payload["pulse"] = num(pulse);
    }
    payload
}

/// Failure writing to Garmin Connect: either the token was rejected (the
/// caller refreshes once and retries), or the write failed for another
/// reason.
#[derive(Debug)]
pub enum WriteFailure {
    Unauthorized,
    Failed(AppError),
}

impl WriteFailure {
    pub fn message(&self) -> &str {
        match self {
            WriteFailure::Unauthorized => "unauthorized (HTTP 401)",
            WriteFailure::Failed(error) => &error.message,
        }
    }
}

/// Garmin rate-limiting: HTTP status 429, or a JSON body whose
/// `error.status-code` is `429` (string or number).
fn is_rate_limited(status: u16, body: &str) -> bool {
    if status == 429 {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json.pointer("/error/status-code").cloned())
        .map(|value| value.as_str() == Some("429") || value.as_u64() == Some(429))
        .unwrap_or(false)
}

/// POST a JSON payload to a Garmin Connect endpoint with the native Android
/// header set and a DI Bearer token. Expects the given success status.
/// `429` (HTTP or JSON-embedded) is retried with backoff; `401` is reported
/// distinctly so the caller can refresh and retry once; `412` is the EU
/// upload-consent gate.
pub fn write_json(
    client: &HttpClient,
    path: &str,
    access_token: &str,
    payload: &serde_json::Value,
    success_status: u16,
) -> Result<(), WriteFailure> {
    let url = client.garmin_api_url(path);
    let mut headers: Vec<(&str, &str)> = Vec::new();
    for (name, value) in native_headers() {
        if name != "Cache-Control" {
            headers.push((name, value));
        }
    }
    let auth = format!("Bearer {access_token}");
    headers.push(("Authorization", auth.as_str()));
    // No explicit Content-Type here: `post_json` sets it via `.json()`.
    // Adding it again would put the header on the wire twice, and Garmin's
    // edge rejects the duplicate with an empty-body 400 (ticket 11).

    let (status, body) = crate::http::retry(
        3,
        |(status, body): &(u16, String)| is_rate_limited(*status, body),
        || {
            client.post_json(&url, payload, &headers).map_err(|error| {
                WriteFailure::Failed(AppError::new(
                    crate::EXIT_METRIC_FAILURE,
                    format!("write to Garmin {path} failed: {error}"),
                ))
            })
        },
    )?;
    if is_rate_limited(status, &body) {
        // Retries exhausted while still rate-limited (possibly a 200 whose
        // JSON body carries a 429) — this write did not succeed.
        return Err(WriteFailure::Failed(AppError::new(
            crate::EXIT_METRIC_FAILURE,
            "Garmin is rate-limiting writes (429); try again later",
        )));
    }
    if status == success_status {
        return Ok(());
    }
    if status == 401 {
        return Err(WriteFailure::Unauthorized);
    }
    if status == 412 {
        return Err(WriteFailure::Failed(AppError::new(
            crate::EXIT_METRIC_FAILURE,
            "Garmin returned HTTP 412 (upload consent required): grant \"upload consent\" \
             in your Garmin Connect account settings (EU accounts need this), then re-run `sync --apply`"
                .to_string(),
        )));
    }
    Err(WriteFailure::Failed(AppError::new(
        crate::EXIT_METRIC_FAILURE,
        format!(
            "write to Garmin {path} failed: HTTP {status}{}",
            if body.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", body.trim())
            }
        ),
    )))
}

/// Write one weigh-in. Success is HTTP 204.
pub fn write_weight(
    client: &HttpClient,
    access_token: &str,
    payload: &serde_json::Value,
) -> Result<(), WriteFailure> {
    write_json(client, WEIGHT_PATH, access_token, payload, 204)
}

/// Write one blood-pressure reading. Success is HTTP 200.
pub fn write_blood_pressure(
    client: &HttpClient,
    access_token: &str,
    payload: &serde_json::Value,
) -> Result<(), WriteFailure> {
    write_json(client, BP_PATH, access_token, payload, 200)
}

/// Result of the SSO login POST.
pub enum LoginOutcome {
    Success { service_ticket: String },
    MfaRequired { method: String },
}

/// Result of one MFA verify attempt.
pub enum MfaOutcome {
    Ticket(String),
    InvalidCode,
}

/// A DI Bearer token pair, including the client id that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub client_id: String,
}

/// Stage A: sign-in page GET (establishes session cookies) + login POST.
pub fn login(
    client: &HttpClient,
    username: &str,
    password: &str,
) -> Result<LoginOutcome, AppError> {
    let sso_headers = sso_headers();
    let (status, _body) = client
        .get(&sign_in_url(&client.base.garmin_sso), &sso_headers)
        .map_err(transport_error)?;
    if !(200..300).contains(&status) {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin SSO sign-in page returned HTTP {status}"),
        ));
    }

    let (status, body) = crate::http::retry(
        3,
        |(status, body): &(u16, String)| is_rate_limited(*status, body),
        || {
            client
                .post_json(
                    &login_url(&client.base.garmin_sso),
                    &login_json(username, password),
                    &sso_headers,
                )
                .map_err(transport_error)
        },
    )?;
    let body = sso_json(status, &body, "login")?;

    let response_type = body
        .pointer("/responseStatus/type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match response_type {
        "SUCCESSFUL" => match body.get("serviceTicketId").and_then(|v| v.as_str()) {
            Some(ticket) => Ok(LoginOutcome::Success {
                service_ticket: ticket.to_string(),
            }),
            None => Err(AppError::new(
                crate::EXIT_AUTH,
                "Garmin login succeeded but returned no serviceTicketId",
            )),
        },
        "MFA_REQUIRED" => {
            let method = body
                .pointer("/customerMfaInfo/mfaLastMethodUsed")
                .and_then(|v| v.as_str())
                .filter(|m| *m == "email" || *m == "totp")
                .unwrap_or("email");
            Ok(LoginOutcome::MfaRequired {
                method: method.to_string(),
            })
        }
        "INVALID_USERNAME_PASSWORD" => Err(AppError::new(
            crate::EXIT_AUTH,
            "Garmin rejected the login: invalid username or password",
        )),
        other => Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin login failed (unexpected response {other})"),
        )),
    }
}

/// One MFA verify attempt. `InvalidCode` means the operator gets to try again.
pub fn verify_mfa(client: &HttpClient, method: &str, code: &str) -> Result<MfaOutcome, AppError> {
    let (status, body) = crate::http::retry(
        3,
        |(status, body): &(u16, String)| is_rate_limited(*status, body),
        || {
            client
                .post_json(
                    &mfa_url(&client.base.garmin_sso),
                    &verify_json(method, code),
                    &sso_headers(),
                )
                .map_err(transport_error)
        },
    )?;
    let body = sso_json(status, &body, "MFA verification")?;

    let response_type = body
        .pointer("/responseStatus/type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match response_type {
        "SUCCESSFUL" => match body.get("serviceTicketId").and_then(|v| v.as_str()) {
            Some(ticket) => Ok(MfaOutcome::Ticket(ticket.to_string())),
            None => Err(AppError::new(
                crate::EXIT_AUTH,
                "Garmin MFA verification succeeded but returned no serviceTicketId",
            )),
        },
        "INVALID_MFA_CODE" | "MFA_FAILED" | "INVALID" => Ok(MfaOutcome::InvalidCode),
        other => Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin MFA verification failed (unexpected response {other})"),
        )),
    }
}

/// Stage B: exchange the CAS service ticket for DI tokens, trying the
/// candidate client ids in order until one succeeds.
pub fn exchange_service_ticket(client: &HttpClient, ticket: &str) -> Result<DiTokens, AppError> {
    let mut last_error: Option<String> = None;
    for candidate in DI_CLIENT_IDS {
        let url = client.garmin_diauth_url(DI_TOKEN_PATH);
        let (status, body) = crate::http::retry(
            3,
            |(status, body): &(u16, String)| is_rate_limited(*status, body),
            || {
                client
                    .post_form_headers(
                        &url,
                        &token_form(candidate, ticket),
                        &native_headers(),
                        Some((candidate, "")),
                    )
                    .map_err(transport_error)
            },
        )?;
        if is_rate_limited(status, &body) {
            // Rate limiting ends the retry walk immediately.
            return Err(AppError::new(
                crate::EXIT_AUTH,
                "Garmin rate-limited the DI token exchange (429); try again later",
            ));
        }
        match parse_di_response(status, &body, candidate) {
            Ok(tokens) => return Ok(tokens),
            Err(error) => last_error = Some(error.message),
        }
    }
    Err(AppError::new(
        crate::EXIT_AUTH,
        format!(
            "DI token exchange failed for all client ids: {}",
            last_error.unwrap_or_else(|| "no response".into())
        ),
    ))
}

/// Stage C: refresh a DI token pair with the stored refresh token. Garmin
/// `429` responses are retried with backoff.
pub fn refresh(
    client: &HttpClient,
    client_id: &str,
    refresh_token: &str,
) -> Result<DiTokens, AppError> {
    if refresh_token.trim().is_empty() {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            "no Garmin refresh token stored; re-run `auth garmin`",
        ));
    }
    let url = client.garmin_diauth_url(DI_TOKEN_PATH);
    let (status, body) = crate::http::retry(
        3,
        |(status, body): &(u16, String)| is_rate_limited(*status, body),
        || {
            client
                .post_form_headers(
                    &url,
                    &refresh_form(client_id, refresh_token),
                    &native_headers(),
                    Some((client_id, "")),
                )
                .map_err(transport_error)
        },
    )?;
    parse_di_response(status, &body, client_id)
}

fn parse_di_response(status: u16, body: &str, client_id: &str) -> Result<DiTokens, AppError> {
    if !(200..300).contains(&status) {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            format!("DI token endpoint returned HTTP {status} for {client_id}"),
        ));
    }
    let json: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        AppError::new(
            crate::EXIT_AUTH,
            format!("DI token endpoint returned invalid JSON: {error}"),
        )
    })?;
    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::new(
                crate::EXIT_AUTH,
                format!("DI token response had no access_token for {client_id}"),
            )
        })?;
    Ok(DiTokens {
        access_token: access_token.to_string(),
        refresh_token: json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        client_id: client_id.to_string(),
    })
}

/// Parse an SSO JSON response, surfacing HTTP status and JSON-embedded
/// `error.status-code` 429s (which may be a string or a number) clearly.
fn sso_json(status: u16, body: &str, what: &str) -> Result<serde_json::Value, AppError> {
    if status == 429 {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin {what} was rate-limited (HTTP 429); try again later"),
        ));
    }
    if !(200..300).contains(&status) {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin {what} returned HTTP {status}"),
        ));
    }
    let json: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin {what} returned invalid JSON: {error}"),
        )
    })?;
    let json_429 = json
        .pointer("/error/status-code")
        .map(|v| v.as_str() == Some("429") || v.as_u64() == Some(429))
        .unwrap_or(false);
    if json_429 {
        return Err(AppError::new(
            crate::EXIT_AUTH,
            format!("Garmin {what} was rate-limited (429 in response body); try again later"),
        ));
    }
    Ok(json)
}

fn transport_error(error: HttpError) -> AppError {
    AppError::new(crate::EXIT_AUTH, format!("could not reach Garmin: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_json_has_exact_shape() {
        assert_eq!(
            login_json("u@example.com", "pw"),
            serde_json::json!({
                "username": "u@example.com",
                "password": "pw",
                "rememberMe": true,
                "captchaToken": ""
            })
        );
    }

    #[test]
    fn verify_json_has_exact_shape() {
        assert_eq!(
            verify_json("totp", "123456"),
            serde_json::json!({
                "mfaMethod": "totp",
                "mfaVerificationCode": "123456",
                "rememberMyBrowser": true,
                "reconsentList": [],
                "mfaSetup": false
            })
        );
    }

    #[test]
    fn token_form_has_exact_fields() {
        assert_eq!(
            token_form("CID", "ST-1"),
            vec![
                ("client_id".into(), "CID".into()),
                ("service_ticket".into(), "ST-1".into()),
                ("grant_type".into(), DI_GRANT_TYPE.into()),
                (
                    "service_url".into(),
                    "https://mobile.integration.garmin.com/gcm/android".into()
                ),
            ]
        );
    }

    #[test]
    fn refresh_form_has_exact_fields() {
        assert_eq!(
            refresh_form("CID", "RT"),
            vec![
                ("grant_type".into(), "refresh_token".into()),
                ("client_id".into(), "CID".into()),
                ("refresh_token".into(), "RT".into()),
            ]
        );
    }

    #[test]
    fn basic_auth_header_is_base64_of_client_id_colon() {
        use base64::Engine;
        let expected = base64::engine::general_purpose::STANDARD
            .encode("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2:");
        assert_eq!(
            basic_auth_header("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"),
            format!("Basic {expected}")
        );
    }

    #[test]
    fn sso_urls_encode_service() {
        assert_eq!(
            sign_in_url("https://sso.garmin.com"),
            "https://sso.garmin.com/mobile/sso/en_US/sign-in?clientId=GCM_ANDROID_DARK&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fandroid"
        );
        assert_eq!(
            login_url("https://sso.garmin.com"),
            "https://sso.garmin.com/mobile/api/login?clientId=GCM_ANDROID_DARK&locale=en-US&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fandroid"
        );
    }

    #[test]
    fn parse_di_response_accepts_valid_tokens() {
        let parsed =
            parse_di_response(200, r#"{"access_token":"at","refresh_token":"rt"}"#, "CID").unwrap();
        assert_eq!(
            parsed,
            DiTokens {
                access_token: "at".into(),
                refresh_token: Some("rt".into()),
                client_id: "CID".into(),
            }
        );
    }

    #[test]
    fn parse_di_response_rejects_missing_access_token() {
        let error = parse_di_response(200, r#"{"error":"invalid_client"}"#, "CID").unwrap_err();
        assert_eq!(error.code, crate::EXIT_AUTH);
        assert!(
            error.message.contains("no access_token"),
            "{}",
            error.message
        );
    }

    #[test]
    fn sso_json_detects_json_embedded_429() {
        let error = sso_json(200, r#"{"error":{"status-code":"429"}}"#, "login").unwrap_err();
        assert!(error.message.contains("429"), "{}", error.message);
        let error = sso_json(200, r#"{"error":{"status-code":429}}"#, "login").unwrap_err();
        assert!(error.message.contains("429"), "{}", error.message);
    }

    #[test]
    fn weight_payload_has_exact_shape() {
        assert_eq!(
            weight_payload("2026-01-02T08:30:00.000", "2026-01-02T08:30:00.000", 82.4),
            serde_json::json!({
                "dateTimestamp": "2026-01-02T08:30:00.000",
                "gmtTimestamp": "2026-01-02T08:30:00.000",
                "unitKey": "kg",
                "sourceType": "MANUAL",
                "value": 82.4
            })
        );
    }

    #[test]
    fn bp_payload_includes_pulse_only_when_present() {
        assert_eq!(
            bp_payload("L", "G", 120.0, 80.0, Some(72.0)),
            serde_json::json!({
                "measurementTimestampLocal": "L",
                "measurementTimestampGMT": "G",
                "systolic": 120,
                "diastolic": 80,
                "pulse": 72,
                "sourceType": "MANUAL"
            })
        );
        let without_pulse = bp_payload("L", "G", 120.0, 80.0, None);
        assert!(without_pulse.get("pulse").is_none());
    }
}
