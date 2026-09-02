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
pub struct AuthArgs {
    /// Config directory override (default: ~/.config/withings-garmin-sync)
    #[arg(long, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Log each API request and response
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct SyncArgs {
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
