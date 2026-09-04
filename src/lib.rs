pub mod cli;
pub mod config;
pub mod garmin;
pub mod http;
pub mod timefmt;
pub mod transform;
pub mod withings;

use std::fs;
use std::io::Write;
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

    // Load the config if one exists, else start from the skeleton. `auth`
    // tolerates empty credentials because it prompts for them next.
    let config_path = config::config_path(&dir);
    let mut config = if config_path.exists() {
        let text = fs::read_to_string(&config_path).map_err(|error| {
            AppError::config(format!("could not read {}: {error}", config_path.display()))
        })?;
        toml::from_str(&text).map_err(|error| {
            AppError::config(format!(
                "invalid config at {}: {error}",
                config_path.display()
            ))
        })?
    } else {
        config::Config::skeleton()
    };

    // One-time interactive Withings OAuth (ticket 05).
    if config.withings.client_id.trim().is_empty() {
        config.withings.client_id =
            prompt_line("Withings client id (from the Withings developer portal): ")?;
    }
    if config.withings.client_secret.trim().is_empty() {
        config.withings.client_secret =
            prompt_line("Withings client secret (from the Withings developer portal): ")?;
    }
    config::write_config(&config_path, &config)?;

    let client = http::HttpClient::new(http::BaseUrls::from_env()).with_verbose(args.verbose);
    if args.verbose {
        eprintln!("[verbose] config dir: {}", dir.display());
        client.log_base_urls();
    }

    let state = withings::generate_state();
    let authorize_url = withings::authorize_url(&config.withings.client_id, &state);
    println!("Open this URL in a browser and authorize the app:");
    println!("{authorize_url}");
    println!(
        "(If the browser reports a bad redirect URI, register this URI in the Withings developer portal: {})",
        withings::REDIRECT_URI
    );
    let _ = std::io::stdout().flush();

    let pasted =
        prompt_line("Paste the redirect URL (or the bare code) your browser was sent to: ")?;
    let code = withings::extract_code(&pasted, &state)?;

    let withings_tokens = withings::exchange_code(
        &client,
        &config.withings.client_id,
        &config.withings.client_secret,
        &code,
    )?;

    // One-time interactive Garmin mobile-SSO login (ticket 06).
    let username = prompt_line("Garmin username (email): ")?;
    let password = prompt_line("Garmin password: ")?;
    let service_ticket = match garmin::login(&client, &username, &password)? {
        garmin::LoginOutcome::Success { service_ticket } => service_ticket,
        garmin::LoginOutcome::MfaRequired { method } => {
            let mut attempts = 0;
            loop {
                attempts += 1;
                if attempts > 3 {
                    return Err(AppError::new(
                        EXIT_AUTH,
                        "too many rejected MFA codes; re-run `auth` to try again",
                    ));
                }
                let code = prompt_line(&format!("Garmin MFA code (sent via {method}): "))?;
                match garmin::verify_mfa(&client, &method, &code)? {
                    garmin::MfaOutcome::Ticket(ticket) => break ticket,
                    garmin::MfaOutcome::InvalidCode => {
                        eprintln!("auth: Garmin rejected that MFA code; try again");
                    }
                }
            }
        }
    };
    let garmin_tokens = garmin::exchange_service_ticket(&client, &service_ticket)?;

    // Only now persist tokens: a failure in either half of `auth` must not
    // leave a partial token file behind.
    let tokens_path = config::tokens_path(&dir);
    let mut tokens = if tokens_path.exists() {
        config::load_tokens(&dir)?
    } else {
        config::Tokens::default()
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    tokens.withings = config::WithingsTokens {
        access_token: withings_tokens.access_token,
        refresh_token: withings_tokens.refresh_token.unwrap_or_default(),
        expires_at: Some(now + withings_tokens.expires_in),
    };
    tokens.garmin = config::GarminTokens {
        access_token: garmin_tokens.access_token,
        refresh_token: garmin_tokens.refresh_token.unwrap_or_default(),
        client_id: Some(garmin_tokens.client_id),
    };
    config::write_tokens(&tokens_path, &tokens)?;

    println!("auth: Withings connected (scope: user.metrics).");
    println!("auth: Garmin connected.");
    println!(
        "auth: tokens stored in {} (plaintext, 0600); keep this file private",
        tokens_path.display()
    );
    Ok(EXIT_OK)
}

/// Mutable Garmin DI token state for one sync run.
struct GarminTokenState {
    access_token: String,
    refresh_token: String,
    client_id: String,
}

/// Write one Garmin record. A `401` triggers a DI token refresh (persisted)
/// followed by a single retry; a rejected refresh is returned as an auth
/// error (`EXIT_AUTH`) so the run aborts with "re-run `auth`".
fn garmin_write<F>(
    client: &http::HttpClient,
    state: &mut GarminTokenState,
    payload: &serde_json::Value,
    tokens_path: &std::path::Path,
    tokens: &mut config::Tokens,
    write: F,
) -> Result<(), AppError>
where
    F: Fn(&http::HttpClient, &str, &serde_json::Value) -> Result<(), garmin::WriteFailure>,
{
    match write(client, &state.access_token, payload) {
        Ok(()) => Ok(()),
        Err(garmin::WriteFailure::Unauthorized) => {
            let refreshed = garmin::refresh(client, &state.client_id, &state.refresh_token)
                .map_err(|error| {
                    AppError::new(
                        EXIT_AUTH,
                        format!(
                            "Garmin token refresh failed: {}; re-run `auth`",
                            error.message
                        ),
                    )
                })?;
            state.access_token = refreshed.access_token;
            if let Some(rotate) = refreshed.refresh_token {
                state.refresh_token = rotate;
            }
            tokens.garmin = config::GarminTokens {
                access_token: state.access_token.clone(),
                refresh_token: state.refresh_token.clone(),
                client_id: Some(state.client_id.clone()),
            };
            config::write_tokens(tokens_path, tokens)?;

            // The single retry with the fresh token.
            match write(client, &state.access_token, payload) {
                Ok(()) => Ok(()),
                Err(failure) => Err(AppError::new(
                    EXIT_METRIC_FAILURE,
                    failure.message().to_string(),
                )),
            }
        }
        Err(failure) => Err(AppError::new(
            EXIT_METRIC_FAILURE,
            failure.message().to_string(),
        )),
    }
}

/// Read one Garmin record. A `401` triggers a DI token refresh (persisted)
/// followed by a single retry; a rejected refresh is returned as an auth
/// error (`EXIT_AUTH`) so the run aborts with "re-run `auth`".
fn garmin_read<F, T>(
    client: &http::HttpClient,
    state: &mut GarminTokenState,
    tokens_path: &std::path::Path,
    tokens: &mut config::Tokens,
    read: F,
) -> Result<T, AppError>
where
    F: Fn(&http::HttpClient, &str) -> Result<T, garmin::WriteFailure>,
{
    match read(client, &state.access_token) {
        Ok(value) => Ok(value),
        Err(garmin::WriteFailure::Unauthorized) => {
            let refreshed = garmin::refresh(client, &state.client_id, &state.refresh_token)
                .map_err(|error| {
                    AppError::new(
                        EXIT_AUTH,
                        format!(
                            "Garmin token refresh failed: {}; re-run `auth`",
                            error.message
                        ),
                    )
                })?;
            state.access_token = refreshed.access_token;
            if let Some(rotate) = refreshed.refresh_token {
                state.refresh_token = rotate;
            }
            tokens.garmin = config::GarminTokens {
                access_token: state.access_token.clone(),
                refresh_token: state.refresh_token.clone(),
                client_id: Some(state.client_id.clone()),
            };
            config::write_tokens(tokens_path, tokens)?;

            // The single retry with the fresh token.
            match read(client, &state.access_token) {
                Ok(value) => Ok(value),
                Err(failure) => Err(AppError::new(
                    EXIT_METRIC_FAILURE,
                    failure.message().to_string(),
                )),
            }
        }
        Err(failure) => Err(AppError::new(
            EXIT_METRIC_FAILURE,
            failure.message().to_string(),
        )),
    }
}

/// Read one line of operator input, prompting on stderr so stdout stays
/// report-only. Empty input (closed stdin) is an auth error.
fn prompt_line(label: &str) -> Result<String, AppError> {
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|error| AppError::new(EXIT_AUTH, format!("could not read input: {error}")))?;
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::new(
            EXIT_AUTH,
            "no input provided (stdin closed); re-run `auth` to continue",
        ));
    }
    Ok(trimmed)
}

fn run_sync(args: SyncArgs) -> Result<i32, AppError> {
    let dir = config::resolve_config_dir(args.config_dir.as_deref())?;

    // Fail fast on missing/invalid config or tokens (exit 3).
    let config = config::load_config(&dir)?;
    let mut tokens = config::load_tokens(&dir)?;
    if tokens.withings.access_token.trim().is_empty()
        || tokens.garmin.access_token.trim().is_empty()
        || tokens
            .garmin
            .client_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(AppError::config(format!(
            "tokens at {} are incomplete; run `auth` first",
            config::tokens_path(&dir).display()
        )));
    }

    let client = http::HttpClient::new(http::BaseUrls::from_env()).with_verbose(args.verbose);
    let dry_run = !args.apply;
    let _ = args.dry_run; // `--dry-run` is the default and conflicts with `--apply`.

    if args.verbose {
        eprintln!("[verbose] config dir: {}", dir.display());
        client.log_base_urls();
    }

    // Refresh an expired Withings access token before reads begin; a rejected
    // refresh is an auth failure (exit 4) and aborts the run.
    let now_epoch = timefmt::now_epoch();
    let expired = tokens
        .withings
        .expires_at
        .map(|expires| expires <= now_epoch as u64 + 60)
        .unwrap_or(true);
    if expired {
        let refreshed = withings::refresh(
            &client,
            &config.withings.client_id,
            &config.withings.client_secret,
            &tokens.withings.refresh_token,
        )
        .map_err(|error| {
            AppError::new(
                EXIT_AUTH,
                format!(
                    "Withings token refresh failed: {}; re-run `auth`",
                    error.message
                ),
            )
        })?;
        tokens.withings.access_token = refreshed.access_token;
        if let Some(rotate) = refreshed.refresh_token {
            tokens.withings.refresh_token = rotate;
        }
        tokens.withings.expires_at = Some(timefmt::now_epoch() as u64 + refreshed.expires_in);
        config::write_tokens(&config::tokens_path(&dir), &tokens)?;
    }

    // Resolve the sync window: --since/--until flags win, then the config's
    // sync.since default, then the built-in last-30-days window.
    let now = timefmt::now_epoch();
    let until_flag = args.until.as_deref().map(timefmt::parse_date).transpose()?;
    let until = until_flag.unwrap_or(now);
    let default_since = if until_flag.is_some() {
        0 // `--until` alone means "from the beginning until ..."
    } else {
        now - 30 * 86400
    };
    let since_flag = args.since.as_deref().map(timefmt::parse_date).transpose()?;
    let since = since_flag.or_else(|| {
        // An explicit `--until` bounds the window by itself; the config's
        // `sync.since` default only applies when no window flag is given.
        if until_flag.is_some() {
            None
        } else {
            config
                .sync
                .since
                .as_deref()
                .map(timefmt::parse_date)
                .transpose()
                .ok()
                .flatten()
        }
    });
    let since = since.unwrap_or(default_since);
    if since > until {
        return Err(AppError::new(
            EXIT_USAGE,
            format!("invalid window: --since {since} is after --until {until}"),
        ));
    }

    let window_label = if since_flag.is_none() && until_flag.is_none() {
        "last 30 days".to_string()
    } else {
        match (since_flag, until_flag) {
            (Some(_), Some(_)) => format!(
                "{}..{}",
                args.since.as_deref().unwrap(),
                args.until.as_deref().unwrap()
            ),
            (Some(_), None) => format!("{}..now", args.since.as_deref().unwrap()),
            (None, Some(_)) => format!("..{}", args.until.as_deref().unwrap()),
            (None, None) => "last 30 days".to_string(),
        }
    };

    // Read the window from Withings once, then split into the two metrics.
    let groups = withings::read_measures(&client, &tokens.withings.access_token, since, until)?;
    let (weights, bps, skips) = transform::transform(groups);

    for skip in &skips {
        eprintln!(
            "warning: skipping {} reading at {}: {}",
            match skip.metric {
                transform::Metric::Weight => "weight",
                transform::Metric::BloodPressure => "blood-pressure",
            },
            timefmt::local_ms(skip.epoch),
            skip.reason
        );
    }

    if dry_run {
        println!(
            "dry-run ({window_label}): would write {} weight and {} blood-pressure measurements",
            weights.len(),
            bps.len()
        );
        for weight in &weights {
            println!(
                "  weight: {} kg at {}",
                weight.kg,
                timefmt::local_ms(weight.epoch)
            );
        }
        for bp in &bps {
            let pulse_part = bp
                .pulse
                .map(|p| format!(" (pulse {p})"))
                .unwrap_or_default();
            println!(
                "  blood-pressure: {}/{}{} at {}",
                bp.systolic,
                bp.diastolic,
                pulse_part,
                timefmt::local_ms(bp.epoch)
            );
        }
        println!("summary: dry-run complete");
        return Ok(EXIT_OK);
    }

    // Apply: write each metric independently; one failing does not block the
    // other. A 401 refreshes the Garmin DI token once and retries.
    let weight_skips = skips
        .iter()
        .filter(|s| s.metric == transform::Metric::Weight)
        .count();
    let bp_skips = skips
        .iter()
        .filter(|s| s.metric == transform::Metric::BloodPressure)
        .count();

    let tokens_path = config::tokens_path(&dir);
    let mut garmin_state = GarminTokenState {
        access_token: tokens.garmin.access_token.clone(),
        refresh_token: tokens.garmin.refresh_token.clone(),
        client_id: tokens.garmin.client_id.clone().unwrap_or_default(),
    };

    // Ticket 12: Garmin does not dedup blood-pressure writes by timestamp,
    // so before writing BP, read back the days Garmin already has and skip
    // any Withings reading on those days. Weight needs no read-back (its
    // endpoint dedups). A failed read-back fails the BP metric closed rather
    // than risk duplicates.
    let mut bp_existing_days: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut bp_readback_failed = false;
    if !bps.is_empty() {
        let start_date = timefmt::local_date(since);
        let end_date = timefmt::local_date(until);
        match garmin_read(
            &client,
            &mut garmin_state,
            &tokens_path,
            &mut tokens,
            |c, t| garmin::read_bp_dates(c, t, &start_date, &end_date),
        ) {
            Ok(dates) => bp_existing_days = dates.into_iter().collect(),
            Err(error) if error.code == EXIT_AUTH => return Err(error),
            Err(error) => {
                eprintln!(
                    "error: blood-pressure read-back failed: {}; skipping all blood-pressure writes this run",
                    error.message
                );
                bp_readback_failed = true;
            }
        }
    }

    let mut weight_failed = 0usize;
    let mut bp_failed = 0usize;
    let mut bp_dedup_skipped = 0usize;
    for weight in &weights {
        let payload = garmin::weight_payload(
            &timefmt::local_ms(weight.epoch),
            &timefmt::gmt_ms(weight.epoch),
            weight.kg,
        );
        match garmin_write(
            &client,
            &mut garmin_state,
            &payload,
            &tokens_path,
            &mut tokens,
            garmin::write_weight,
        ) {
            Ok(()) => {}
            Err(error) if error.code == EXIT_AUTH => return Err(error),
            Err(error) => {
                eprintln!("error: weight write failed: {}", error.message);
                weight_failed += 1;
            }
        }
    }
    for bp in &bps {
        if bp_readback_failed {
            bp_failed += 1;
            continue;
        }
        // Skip re-writes of days Garmin already has (ticket 12): the BP
        // endpoint does not dedup by timestamp, so identical re-writes would
        // duplicate. Day granularity is the finest the read-back exposes.
        let day = timefmt::local_date(bp.epoch);
        if bp_existing_days.contains(&day) {
            eprintln!(
                "warning: skipping blood-pressure reading at {}: {} already on Garmin",
                timefmt::local_ms(bp.epoch),
                day
            );
            bp_dedup_skipped += 1;
            continue;
        }
        let payload = garmin::bp_payload(
            &timefmt::local_ms(bp.epoch),
            &timefmt::gmt_ms(bp.epoch),
            bp.systolic,
            bp.diastolic,
            bp.pulse,
        );
        match garmin_write(
            &client,
            &mut garmin_state,
            &payload,
            &tokens_path,
            &mut tokens,
            garmin::write_blood_pressure,
        ) {
            Ok(()) => {}
            Err(error) if error.code == EXIT_AUTH => return Err(error),
            Err(error) => {
                eprintln!("error: blood-pressure write failed: {}", error.message);
                bp_failed += 1;
            }
        }
    }

    let weight_written = weights.len().saturating_sub(weight_failed);
    let bp_written = bps
        .len()
        .saturating_sub(bp_failed)
        .saturating_sub(bp_dedup_skipped);
    println!(
        "apply ({window_label}): weight: {weight_written} written, {weight_skips} skipped, {weight_failed} failed"
    );
    println!(
        "apply ({window_label}): blood-pressure: {bp_written} written, {} skipped, {bp_failed} failed",
        bp_skips + bp_dedup_skipped
    );

    if weight_failed > 0 || bp_failed > 0 {
        println!("summary: 1 metric(s) failed");
        Ok(EXIT_METRIC_FAILURE)
    } else {
        println!("summary: all metrics synced");
        Ok(EXIT_OK)
    }
}
