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
    #[serde(
        default,
        skip_serializing_if = "MetricFloor::is_empty",
        deserialize_with = "floor::deserialize_weight"
    )]
    pub weight: MetricFloor,
    #[serde(
        default,
        skip_serializing_if = "MetricFloor::is_empty",
        deserialize_with = "floor::deserialize_bp"
    )]
    pub bp: MetricFloor,
}

/// One metric's machine-updated sync floor (ADR-0008): the epoch-second
/// Withings timestamp of the newest measurement already written to Garmin.
/// Absent = the metric has no floor yet and bootstraps from the rolling
/// window. The legacy shared `sync.since` key is no longer part of the
/// schema; if present in an existing file it is ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricFloor {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "floor")]
    pub since: Option<i64>,
}

/// Serde boundary for `MetricFloor.since` (ADR-0009): the floor stays an
/// epoch-second `i64` internally and (de)serializes as the canonical
/// RFC 3339 UTC string. Integers are the legacy ADR-0008 format and still
/// load; the next `write_config` emits the canonical string.
mod floor {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::MetricFloor;
    use crate::timefmt::{format_floor, parse_floor};

    /// Serialize `Some(epoch)` as the canonical RFC 3339 UTC string.
    pub fn serialize<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(epoch) => serializer.serialize_str(&format_floor(*epoch)),
            None => serializer.serialize_none(),
        }
    }

    /// Deserialize `Option<i64>` from a canonical or hand-edited string, or
    /// a legacy integer.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Option<i64>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "an RFC 3339 UTC datetime, a YYYY-MM-DD date, or an epoch-second integer",
                )
            }

            fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
            where
                D2: Deserializer<'de>,
            {
                deserializer.deserialize_any(Visitor)
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                parse_floor(&value.to_string()).map(Some).map_err(E::custom)
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                parse_floor(&value.to_string()).map(Some).map_err(E::custom)
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                parse_floor(value).map(Some).map_err(E::custom)
            }
        }
        deserializer.deserialize_option(Visitor)
    }

    /// Deserialize `sync.weight`, naming the config key in errors.
    pub fn deserialize_weight<'de, D>(deserializer: D) -> Result<MetricFloor, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_named("sync.weight.since", deserializer)
    }

    /// Deserialize `sync.bp`, naming the config key in errors.
    pub fn deserialize_bp<'de, D>(deserializer: D) -> Result<MetricFloor, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_named("sync.bp.since", deserializer)
    }

    fn deserialize_named<'de, D>(key: &str, deserializer: D) -> Result<MetricFloor, D::Error>
    where
        D: Deserializer<'de>,
    {
        MetricFloor::deserialize(deserializer)
            .map_err(|error| D::Error::custom(format!("invalid {key}: {error}")))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-01-02T08:30:00Z.
    const EPOCH: i64 = 1767342600;
    /// 2026-01-02T00:00:00Z.
    const MIDNIGHT: i64 = 1767312000;

    fn parse(text: &str) -> Config {
        let full = format!(
            "[withings]\nclient_id = \"test\"\nclient_secret = \"test\"\n{text}"
        );
        toml::from_str(&full).unwrap()
    }

    #[test]
    fn canonical_iso_floor_loads_and_round_trips() {
        let config = parse("[sync.weight]\nsince = \"2026-01-02T08:30:00Z\"\n");
        assert_eq!(config.sync.weight.since, Some(EPOCH));
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("since = \"2026-01-02T08:30:00Z\""),
            "text: {text}"
        );
        let again: Config = toml::from_str(&text).unwrap();
        assert_eq!(again.sync.weight.since, Some(EPOCH));
    }

    #[test]
    fn legacy_integer_floor_loads_and_serializes_canonically() {
        let config = parse("[sync.bp]\nsince = 1767342600\n");
        assert_eq!(config.sync.bp.since, Some(EPOCH));
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("since = \"2026-01-02T08:30:00Z\""),
            "text: {text}"
        );
    }

    #[test]
    fn hand_edited_forms_load_and_normalize_on_write() {
        let config = parse(concat!(
            "[sync.weight]\nsince = \"2026-01-02T10:30:00+02:00\"\n",
            "[sync.bp]\nsince = \"2026-01-02T08:30:00.999Z\"\n",
        ));
        // Offset converts to the UTC instant; fraction truncates toward the
        // floor.
        assert_eq!(config.sync.weight.since, Some(EPOCH));
        assert_eq!(config.sync.bp.since, Some(EPOCH));
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("since = \"2026-01-02T08:30:00Z\""),
            "text: {text}"
        );

        // A plain date is midnight UTC.
        let config = parse("[sync.bp]\nsince = \"2026-01-02\"\n");
        assert_eq!(config.sync.bp.since, Some(MIDNIGHT));
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("since = \"2026-01-02T00:00:00Z\""),
            "text: {text}"
        );
    }

    #[test]
    fn malformed_floor_names_the_config_key_and_accepted_formats() {
        for (metric, value) in [
            ("bp", "junk"),
            ("bp", "2026-01-02T08:30:00"),      // naive, no timezone
            ("weight", "2026-01-02 08:30:00Z"), // space separator
        ] {
            let error = toml::from_str::<Config>(&format!(
                "[withings]\nclient_id = \"test\"\nclient_secret = \"test\"\n\
                 [sync.{metric}]\nsince = {value:?}\n"
            ))
            .unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains(&format!("sync.{metric}.since")),
                "message for {metric}/{value:?}: {message}"
            );
            assert!(
                message.contains("RFC 3339"),
                "message for {metric}/{value:?}: {message}"
            );
        }
    }

    #[test]
    fn absent_floors_still_serialize_away() {
        let config = parse("");
        assert_eq!(config.sync.weight.since, None);
        assert_eq!(config.sync.bp.since, None);
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(!text.contains("since"), "text: {text}");
        assert!(!text.contains("[sync.weight]"), "text: {text}");
        assert!(!text.contains("[sync.bp]"), "text: {text}");
    }

    #[test]
    fn legacy_shared_sync_since_still_loads_and_is_ignored() {
        let config = parse("[sync]\nsince = \"2026-01-15\"\n");
        assert_eq!(config.sync.weight.since, None);
        assert_eq!(config.sync.bp.since, None);
    }
}
