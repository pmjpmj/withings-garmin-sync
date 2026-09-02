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
/// jar, for the Garmin SSO handshake).
#[derive(Clone, Debug)]
pub struct HttpClient {
    pub base: BaseUrls,
    inner: reqwest::blocking::Client,
}

impl HttpClient {
    pub fn new(base: BaseUrls) -> Self {
        let inner = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("build HTTP client");
        Self { base, inner }
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
        let mut request = self.inner.post(url).json(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.send(request, url)
    }

    /// GET a URL with extra headers.
    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), HttpError> {
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
        let response = request
            .send()
            .map_err(|error| HttpError::Transport(format!("{url}: {error}")))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|error| HttpError::Transport(format!("{url}: {error}")))?;
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
                    write!(f, ": {}", body.trim())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for HttpError {}

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
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}
