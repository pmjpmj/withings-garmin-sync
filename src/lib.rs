pub mod cli;
pub mod config;
pub mod http;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use cli::{AuthArgs, Command, SyncArgs};

pub const EXIT_OK: i32 = 0;
pub const EXIT_METRIC_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CONFIG: i32 = 3;
pub const EXIT_AUTH: i32 = 4;

#[derive(Debug)]
pub struct AppError {
    pub code: i32,
    pub message: String,
}

impl AppError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(EXIT_CONFIG, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

/// Run a parsed command, returning the process exit code or a terminal error.
pub fn run(cli: cli::Cli) -> Result<i32, AppError> {
    match cli.command {
        Command::Auth(args) => run_auth(args),
        Command::Sync(args) => run_sync(args),
    }
}

fn run_auth(args: AuthArgs) -> Result<i32, AppError> {
    let dir = config::resolve_config_dir(args.config_dir.as_deref())?;
    fs::create_dir_all(&dir).map_err(|error| {
        AppError::config(format!("could not create {}: {error}", dir.display()))
    })?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        AppError::config(format!(
            "could not set 0700 permissions on {}: {error}",
            dir.display()
        ))
    })?;

    let config_file = config::config_path(&dir);
    let tokens_file = config::tokens_path(&dir);

    let mut created: Vec<&str> = Vec::new();
    if !config_file.exists() {
        config::write_config(&config_file, &config::Config::skeleton())?;
        created.push("config.toml");
    }
    if !tokens_file.exists() {
        config::write_tokens(&tokens_file, &config::Tokens::default())?;
        created.push("tokens.json");
    }

    if created.is_empty() {
        println!("auth: config and tokens already exist at {}", dir.display());
    } else {
        println!("auth: wrote {} to {}", created.join(", "), dir.display());
    }
    println!("auth: interactive Withings + Garmin auth lands in tickets 05 and 06");

    let _ = args.verbose;
    Ok(EXIT_OK)
}

fn run_sync(args: SyncArgs) -> Result<i32, AppError> {
    let dir = config::resolve_config_dir(args.config_dir.as_deref())?;

    // Fail fast on missing/invalid config or tokens (exit 3).
    config::load_config(&dir)?;
    config::load_tokens(&dir)?;

    let client = http::HttpClient::new(http::BaseUrls::from_env());
    let dry_run = !args.apply;
    let _ = args.dry_run; // `--dry-run` is the default and conflicts with `--apply`.

    if args.verbose {
        eprintln!("[verbose] config dir: {}", dir.display());
        eprintln!("[verbose] HTTP base URLs:");
        eprintln!("[verbose]   withings_api  = {}", client.base.withings_api);
        eprintln!("[verbose]   garmin_sso    = {}", client.base.garmin_sso);
        eprintln!("[verbose]   garmin_diauth = {}", client.base.garmin_diauth);
        eprintln!("[verbose]   garmin_api    = {}", client.base.garmin_api);
    }

    let window = match (args.since.as_deref(), args.until.as_deref()) {
        (Some(since), Some(until)) => format!("{since}..{until}"),
        (Some(since), None) => format!("{since}..now"),
        (None, Some(until)) => format!("..{until}"),
        (None, None) => "last 30 days".to_string(),
    };

    if dry_run {
        println!("dry-run ({window}): would write 0 weight and 0 blood-pressure measurements");
    } else {
        println!("apply ({window}): wrote 0 weight and 0 blood-pressure measurements");
    }

    Ok(EXIT_OK)
}
