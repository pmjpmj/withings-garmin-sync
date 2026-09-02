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

/// Owns the configured base URLs. Later tickets add the real request methods.
#[derive(Clone, Debug)]
pub struct HttpClient {
    pub base: BaseUrls,
}

impl HttpClient {
    pub fn new(base: BaseUrls) -> Self {
        Self { base }
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

fn join(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}
