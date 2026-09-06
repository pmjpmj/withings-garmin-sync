use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::AppError;

pub const CONFIG_FILE: &str = "config.toml";
pub const TOKENS_FILE: &str = "tokens.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub withings: WithingsConfig,
    #[serde(default)]
    pub sync: SyncConfig,
}

impl Config {
    /// The starting config `auth` seeds when none exists yet: empty Withings
    /// credentials (prompted for by `auth`) and default sync settings.
    pub fn skeleton() -> Self {
        Self {
            withings: WithingsConfig {
                client_id: String::new(),
                client_secret: String::new(),
            },
            sync: SyncConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithingsConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default, skip_serializing_if = "MetricFloor::is_empty")]
    pub weight: MetricFloor,
    #[serde(default, skip_serializing_if = "MetricFloor::is_empty")]
    pub bp: MetricFloor,
}

/// One metric's machine-updated sync floor (ADR-0008): the epoch-second
/// Withings timestamp of the newest measurement already written to Garmin.
/// Absent = the metric has no floor yet and bootstraps from the rolling
/// window. The legacy shared `sync.since` key is no longer part of the
/// schema; if present in an existing file it is ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricFloor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
}

impl MetricFloor {
    /// An empty floor serializes away entirely, so a config with no floors
    /// yet has no `[sync.weight]`/`[sync.bp]` tables.
    fn is_empty(&self) -> bool {
        self.since.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default)]
    pub withings: WithingsTokens,
    #[serde(default)]
    pub garmin: GarminTokens,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WithingsTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// Access-token expiry as Unix epoch seconds (computed at exchange/refresh
    /// time from Withings' `expires_in`), so `sync` can refresh before reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GarminTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

/// Resolve the config directory: `--config-dir` wins, otherwise
/// `~/.config/withings-garmin-sync`.
pub fn resolve_config_dir(override_path: Option<&Path>) -> Result<PathBuf, AppError> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        AppError::config("HOME is not set; pass --config-dir to choose a config directory")
    })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("withings-garmin-sync"))
}

pub fn config_path(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE)
}

pub fn tokens_path(dir: &Path) -> PathBuf {
    dir.join(TOKENS_FILE)
}

pub fn load_config(dir: &Path) -> Result<Config, AppError> {
    let path = config_path(dir);
    let text = fs::read_to_string(&path).map_err(|_| {
        AppError::config(format!(
            "config not found at {}; run `auth` first",
            path.display()
        ))
    })?;

    let config: Config = toml::from_str(&text).map_err(|error| {
        AppError::config(format!(
            "invalid config at {}: {}; run `auth` first",
            path.display(),
            error
        ))
    })?;

    if config.withings.client_id.trim().is_empty()
        || config.withings.client_secret.trim().is_empty()
    {
        return Err(AppError::config(format!(
            "invalid config at {}: withings.client_id and withings.client_secret are required; run `auth` first",
            path.display()
        )));
    }

    Ok(config)
}

pub fn load_tokens(dir: &Path) -> Result<Tokens, AppError> {
    let path = tokens_path(dir);
    let text = fs::read_to_string(&path).map_err(|_| {
        AppError::config(format!(
            "tokens not found at {}; run `auth` first",
            path.display()
        ))
    })?;

    let tokens: Tokens = serde_json::from_str(&text).map_err(|error| {
        AppError::config(format!(
            "invalid tokens at {}: {}; run `auth` first",
            path.display(),
            error
        ))
    })?;

    Ok(tokens)
}

pub fn write_config(path: &Path, config: &Config) -> Result<(), AppError> {
    let text = toml::to_string_pretty(config)
        .map_err(|error| AppError::config(format!("could not serialize config: {error}")))?;
    write_private(path, text.as_bytes())
}

pub fn write_tokens(path: &Path, tokens: &Tokens) -> Result<(), AppError> {
    let text = serde_json::to_string_pretty(tokens)
        .map_err(|error| AppError::config(format!("could not serialize tokens: {error}")))?;
    write_private(path, text.as_bytes())
}

/// Write bytes to `path`, creating/truncating it, and force `0600` permissions.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);

    let mut file = options.open(path).map_err(|error| {
        AppError::config(format!("could not write {}: {error}", path.display()))
    })?;
    file.write_all(bytes).map_err(|error| {
        AppError::config(format!("could not write {}: {error}", path.display()))
    })?;
    drop(file);

    set_private_permissions(path)
}

fn set_private_permissions(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        AppError::config(format!(
            "could not set 0600 permissions on {}: {error}",
            path.display()
        ))
    })
}
