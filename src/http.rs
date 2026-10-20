//! The single test seam: injectable HTTP base URLs.
//!
//! Every external host the CLI talks to is overridable through an environment
//! variable. Tests point these at local fake servers; production defaults point
//! at the real endpoints.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseUrls {
    pub withings_api: String,
    pub garmin_sso: String,
    pub garmin_diauth: String,
    pub garmin_api: String,
}

impl BaseUrls {
    pub fn from_env() -> Self {
        Self {
            withings_api: env_base("WGS_WITHINGS_API_BASE", "https://wbsapi.withings.net"),
            garmin_sso: env_base("WGS_GARMIN_SSO_BASE", "https://sso.garmin.com"),
            garmin_diauth: env_base("WGS_GARMIN_DIAUTH_BASE", "https://diauth.garmin.com"),
            garmin_api: env_base("WGS_GARMIN_API_BASE", "https://connectapi.garmin.com"),
        }
    }
}

fn env_base(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Owns the configured base URLs and the shared HTTP client (with a cookie
/// jar, for the Garmin SSO handshake). When `verbose` is set, every request
/// and response is logged to stderr.
#[derive(Clone, Debug)]
pub struct HttpClient {
    pub base: BaseUrls,
    inner: reqwest::blocking::Client,
    verbose: bool,
}

impl HttpClient {
    pub fn new(base: BaseUrls) -> Self {
        let inner = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("build HTTP client");
        Self { base, inner, verbose: false }
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    fn log_request(&self, method: &str, url: &str, headers: &[(&str, &str)], body: &str) {
        if !self.verbose {
            return;
        }
        eprintln!("[verbose] -> {method} {url}");
        for (name, value) in headers {
            eprintln!("[verbose]    {}", redact_header(name, value));
        }
        if !body.is_empty() {
            let preview: String = redact_body(body).chars().take(300).collect();
            eprintln!("[verbose]    body: {preview}");
        }
    }

    /// Log the configured base URLs in verbose mode.
    pub fn log_base_urls(&self) {
        if !self.verbose {
            return;
        }
        eprintln!("[verbose] HTTP base URLs:");
        eprintln!("[verbose]   withings_api  = {}", self.base.withings_api);
        eprintln!("[verbose]   garmin_sso    = {}", self.base.garmin_sso);
        eprintln!("[verbose]   garmin_diauth = {}", self.base.garmin_diauth);
        eprintln!("[verbose]   garmin_api    = {}", self.base.garmin_api);
    }

    /// POST `application/x-www-form-urlencoded` fields to `url`; returns the
    /// response status and body text. Transport errors are surfaced as-is.
    pub fn post_form(
        &self,
        url: &str,
        fields: &[(String, String)],
    ) -> Result<(u16, String), HttpError> {
        self.post_form_headers(url, fields, &[], None)
    }

    /// POST form fields with extra headers and optional HTTP Basic auth.
    pub fn post_form_headers(
        &self,
        url: &str,
        fields: &[(String, String)],
        headers: &[(&str, &str)],
        basic_auth: Option<(&str, &str)>,
    ) -> Result<(u16, String), HttpError> {
        let body: String =
            fields.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        self.log_request("POST", url, headers, &body);
        let mut request = self.inner.post(url).form(fields);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some((user, password)) = basic_auth {
            request = request.basic_auth(user, Some(password));
        }
        self.send(request, url)
    }

    /// POST a JSON body with extra headers.
    pub fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        headers: &[(&str, &str)],
    ) -> Result<(u16, String), HttpError> {
        let body_text = serde_json::to_string(body).unwrap_or_default();
        self.log_request("POST", url, headers, &body_text);
        let mut request = self.inner.post(url).json(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.send(request, url)
    }

    /// GET a URL with extra headers.
    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), HttpError> {
        self.log_request("GET", url, headers, "");
        let mut request = self.inner.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.send(request, url)
    }

    fn send(
        &self,
        request: reqwest::blocking::RequestBuilder,
        url: &str,
    ) -> Result<(u16, String), HttpError> {
        let response =
            request.send().map_err(|error| HttpError::Transport(format!("{url}: {error}")))?;
        let status = response.status().as_u16();
        let version = response.version();
        let response_headers: Vec<String> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                redact_header(name.as_str(), value.to_str().unwrap_or("<non-utf8 value>"))
            })
            .collect();
        let body =
            response.text().map_err(|error| HttpError::Transport(format!("{url}: {error}")))?;
        if self.verbose {
            let preview: String = redact_body(&body).chars().take(300).collect();
            eprintln!("[verbose] <- HTTP {status} ({version:?}) {url}");
            for header in &response_headers {
                eprintln!("[verbose]    {header}");
            }
            if !preview.trim().is_empty() {
                eprintln!("[verbose]    body: {preview}");
            }
        }
        Ok((status, body))
    }

    pub fn withings_url(&self, path: &str) -> String {
        join(&self.base.withings_api, path)
    }

    pub fn garmin_sso_url(&self, path: &str) -> String {
        join(&self.base.garmin_sso, path)
    }

    pub fn garmin_diauth_url(&self, path: &str) -> String {
        join(&self.base.garmin_diauth, path)
    }

    pub fn garmin_api_url(&self, path: &str) -> String {
        join(&self.base.garmin_api, path)
    }
}

/// Failure talking to a remote host: either the transport itself failed, or
/// the server answered with a non-success HTTP status.
#[derive(Debug)]
pub enum HttpError {
    Transport(String),
    Status(u16, String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Transport(message) => write!(f, "{message}"),
            HttpError::Status(status, body) => {
                write!(f, "HTTP {status}")?;
                if !body.trim().is_empty() {
                    write!(f, ": {}", redact_body(body).trim())?;
                }
                Ok(())
            }
        }
    }
}

const SECRET_FIELDS: &[&str] =
    &["password", "client_secret", "access_token", "refresh_token", "service_ticket", "code"];

fn redact_header(name: &str, value: &str) -> String {
    if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("set-cookie") {
        format!("{name}: [REDACTED]")
    } else {
        format!("{name}: {value}")
    }
}

/// Scrub only the printable copy. Request and response bodies passed to
/// protocol code remain untouched.
pub(crate) fn redact_body(body: &str) -> String {
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) {
        redact_json(&mut value);
        return serde_json::to_string(&value).unwrap_or_else(|_| body.to_string());
    }

    // Form bodies are represented as key=value pairs. Preserve their original
    // encoding and replace only values for explicitly named secret fields.
    if body.contains('=') {
        return body
            .split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                if SECRET_FIELDS.contains(&decode_form_component(key).as_str()) {
                    format!("{key}=[REDACTED]")
                } else if pair.contains('=') {
                    format!("{key}={value}")
                } else {
                    pair.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("&");
    }
    body.to_string()
}

fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                // Garmin names the CAS ticket `serviceTicketId` in its SSO
                // response; it is the same secret later sent as
                // `service_ticket` in the token exchange.
                if SECRET_FIELDS.contains(&key.as_str()) || key == "serviceTicketId" {
                    *value = serde_json::Value::String("[REDACTED]".into());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json(value);
            }
        }
        _ => {}
    }
}

fn decode_form_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                decoded.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    decoded.push(a * 16 + b);
                    i += 3;
                } else {
                    decoded.push(bytes[i]);
                    i += 1;
                }
            }
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl std::error::Error for HttpError {}

/// Exponential backoff sleep: 200ms * 2^attempt. Kept small so tests stay
/// fast while still giving a busy server a moment to recover.
pub fn backoff(attempt: u32) {
    let millis = 200u64.saturating_mul(2u64.saturating_pow(attempt.min(6)));
    std::thread::sleep(std::time::Duration::from_millis(millis));
}

/// Run `operation` up to `attempts` times, sleeping with backoff before each
/// retry while `should_retry(&result)` is true. The final attempt's result is
/// returned as-is for the caller to report.
pub fn retry<T, E>(
    attempts: u32,
    should_retry: impl Fn(&T) -> bool,
    mut operation: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let mut attempt: u32 = 0;
    loop {
        let result = operation()?;
        if should_retry(&result) && attempt + 1 < attempts {
            attempt += 1;
            backoff(attempt);
            continue;
        }
        return Ok(result);
    }
}

/// Percent-encode a value for a URL query string: space as `%20`, everything
/// outside unreserved characters encoded.
pub fn query_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

#[cfg(test)]
mod redaction_tests {
    use super::{HttpError, redact_body, redact_header};

    #[test]
    fn redacts_form_fields_and_preserves_safe_fields() {
        assert_eq!(
            redact_body("client_id=abc&client_secret=shh&code=one"),
            "client_id=abc&client_secret=[REDACTED]&code=[REDACTED]"
        );
    }

    #[test]
    fn redacts_nested_json_secrets() {
        assert_eq!(
            redact_body(r#"{"access_token":"a","nested":[{"refresh_token":"r"}],"state":"ok"}"#),
            r#"{"access_token":"[REDACTED]","nested":[{"refresh_token":"[REDACTED]"}],"state":"ok"}"#
        );
    }

    #[test]
    fn leaves_non_json_non_form_text_alone() {
        assert_eq!(redact_body("service_ticket blah"), "service_ticket blah");
    }

    #[test]
    fn redacts_sensitive_headers_case_insensitively() {
        assert_eq!(redact_header("authorization", "Bearer secret"), "authorization: [REDACTED]");
        assert_eq!(redact_header("SET-cookie", "sid=secret"), "SET-cookie: [REDACTED]");
        assert_eq!(redact_header("x-safe", "value"), "x-safe: value");
    }

    #[test]
    fn status_error_display_redacts_json_body() {
        assert_eq!(
            HttpError::Status(401, r#"{"access_token":"secret"}"#.into()).to_string(),
            "HTTP 401: {\"access_token\":\"[REDACTED]\"}"
        );
    }
}
