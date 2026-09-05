use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "withings-garmin-sync",
    version,
    about = "Sync Withings body weight and blood pressure into Garmin Connect"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Interactive one-time authentication with Withings and Garmin
    Auth(AuthArgs),
    /// Read Withings measurements and write them to Garmin (dry-run by default)
    Sync(SyncArgs),
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct AuthArgs {
    /// Service to authenticate; bare `auth` authenticates both (ADR-0006)
    #[command(subcommand)]
    pub service: Option<AuthService>,

    #[command(flatten)]
    pub options: AuthOptions,
}

#[derive(Subcommand)]
pub enum AuthService {
    /// Withings OAuth only (browser authorization-code flow)
    Withings(AuthOptions),
    /// Garmin mobile-SSO login only (username/password/MFA)
    Garmin(AuthOptions),
}

/// Which service(s) one auth run targets, resolved from [`AuthService`]
/// (ADR-0006): a subcommand names one service, bare `auth` is both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthServiceKind {
    Withings,
    Garmin,
    Both,
}

impl AuthService {
    /// Split a parsed subcommand into its flags and the service it selects.
    pub fn into_parts(self) -> (AuthOptions, AuthServiceKind) {
        match self {
            AuthService::Withings(options) => (options, AuthServiceKind::Withings),
            AuthService::Garmin(options) => (options, AuthServiceKind::Garmin),
        }
    }
}

#[derive(Args)]
pub struct AuthOptions {
    /// Config directory override (default: ~/.config/withings-garmin-sync)
    #[arg(long, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Log each API request and response
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct SyncArgs {
    /// Metric to sync; bare `sync` is the same as `sync all`
    #[command(subcommand)]
    pub metric: Option<SyncMetric>,

    #[command(flatten)]
    pub options: SyncOptions,
}

#[derive(Subcommand)]
pub enum SyncMetric {
    /// Sync body weight only
    Weight(SyncOptions),
    /// Sync blood-pressure only
    Bp(SyncOptions),
    /// Sync both metrics (the default)
    All(SyncOptions),
}

/// The metric scope of one sync run, resolved from [`SyncMetric`] (ADR-0005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricScope {
    Weight,
    Bp,
    All,
}

impl SyncMetric {
    /// Split a parsed subcommand into its flags and the scope it selects.
    pub fn into_parts(self) -> (SyncOptions, MetricScope) {
        match self {
            SyncMetric::Weight(options) => (options, MetricScope::Weight),
            SyncMetric::Bp(options) => (options, MetricScope::Bp),
            SyncMetric::All(options) => (options, MetricScope::All),
        }
    }
}

#[derive(Args)]
pub struct SyncOptions {
    /// Write to Garmin Connect (default is a dry run)
    #[arg(long)]
    pub apply: bool,

    /// Dry run: print what would be written without writing (the default)
    #[arg(long, conflicts_with = "apply")]
    pub dry_run: bool,

    /// Only sync measurements on or after this date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,

    /// Only sync measurements before this date (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    pub until: Option<String>,

    /// Config directory override (default: ~/.config/withings-garmin-sync)
    #[arg(long, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Log each API request and response
    #[arg(long)]
    pub verbose: bool,
}
