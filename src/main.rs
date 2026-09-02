use std::process::ExitCode;

use clap::Parser;
use withings_garmin_sync::cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match withings_garmin_sync::run(cli) {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("error: {}", error.message);
            ExitCode::from(error.code as u8)
        }
    }
}
