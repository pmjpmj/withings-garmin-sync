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
use std::path::Path;

use cli::{AuthArgs, AuthServiceKind, Command, MetricScope, SyncArgs};

pub const EXIT_OK: i32 = 0;
pub const EXIT_METRIC_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CONFIG: i32 = 3;
pub const EXIT_AUTH: i32 = 4;

/// Built-in sync window when neither --since/--until nor `sync.since` are
/// set: the rolling last 24 hours (ADR-0001).
pub const DEFAULT_WINDOW_SECS: i64 = 86400;

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

/// Scope membership and the Withings meastype mapping for one run
/// (ADR-0005).
impl MetricScope {
    /// The Withings meastype set this scope requests.
    fn meastypes(self) -> &'static str {
        match self {
            MetricScope::Weight => withings::WEIGHT_MEASTYPES,
            MetricScope::Bp => withings::BP_MEASTYPES,
            MetricScope::All => withings::ALL_MEASTYPES,
        }
    }

    /// True when this run may touch the weight path.
    fn includes_weight(self) -> bool {
        self != MetricScope::Bp
    }

    /// True when this run may touch the blood-pressure path.
    fn includes_bp(self) -> bool {
        self != MetricScope::Weight
    }
}

/// One metric's resolved lower bound for a sync run (ADR-0008).
struct MetricBound {
    /// The Withings `startdate` this metric's window needs: `floor + 1` when
    /// floored (strictly newer), else the flag or rolling value.
    epoch: i64,
    /// Client-side keep filter: drop readings with `epoch < keep_from`.
    /// Always set for floor-derived bounds (so a fake or real server never
    /// re-delivers the last-written measurement); rolling bounds filter only
    /// when a combined read started earlier than them.
    keep_from: Option<i64>,
}

impl MetricBound {
    fn resolve(
        floor: Option<i64>,
        since_flag: Option<i64>,
        until_flag: Option<i64>,
        now: i64,
    ) -> Self {
        if let Some(since) = since_flag {
            // An explicit flag bounds the window by itself (ADR-0001); the
            // floor plays no part and nothing is filtered client-side.
            return MetricBound {
                epoch: since,
                keep_from: None,
            };
        }
        if until_flag.is_some() {
            // `--until` alone means "from the beginning" (ADR-0001).
            return MetricBound {
                epoch: 0,
                keep_from: None,
            };
        }
        match floor {
            Some(floor) => MetricBound {
                epoch: floor + 1,
                keep_from: Some(floor + 1),
            },
            None => MetricBound {
                epoch: now - DEFAULT_WINDOW_SECS,
                keep_from: None,
            },
        }
    }
}

/// The flag-free report label for one metric's bound: its floor, or the
/// rolling bootstrap.
fn floor_label(name: &str, floor: Option<i64>) -> String {
    match floor {
        Some(floor) => format!("{name} floor {floor}"),
        None => "last 24 hours".to_string(),
    }
}

/// Run a parsed command, returning the process exit code or a terminal error.
pub fn run(cli: cli::Cli) -> Result<i32, AppError> {
    match cli.command {
        Command::Auth(args) => run_auth(args),
        Command::Sync(args) => run_sync(args),
    }
}

fn run_auth(args: AuthArgs) -> Result<i32, AppError> {
    // Resolve the target service: `auth withings` / `auth garmin` run one
    // service alone; bare `auth` runs both, back-compatible (ADR-0006).
    let (options, service) = match args.service {
        Some(service) => service.into_parts(),
        None => (args.options, AuthServiceKind::Both),
    };

    let dir = config::resolve_config_dir(options.config_dir.as_deref())?;
    fs::create_dir_all(&dir).map_err(|error| {
        AppError::config(format!("could not create {}: {error}", dir.display()))
    })?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        AppError::config(format!(
            "could not set 0700 permissions on {}: {error}",
            dir.display()
        ))
    })?;

    let client = http::HttpClient::new(http::BaseUrls::from_env()).with_verbose(options.verbose);
    if options.verbose {
        eprintln!("[verbose] config dir: {}", dir.display());
        client.log_base_urls();
    }

    match service {
        AuthServiceKind::Both => {
            run_auth_withings(&client, &dir)?;
            run_auth_garmin(&client, &dir)?;
        }
        AuthServiceKind::Withings => run_auth_withings(&client, &dir)?,
        AuthServiceKind::Garmin => run_auth_garmin(&client, &dir)?,
    }

    println!(
        "auth: tokens stored in {} (plaintext, 0600); keep this file private",
        config::tokens_path(&dir).display()
    );
    Ok(EXIT_OK)
}

/// The Withings half of `auth`: credentials prompt (when missing), browser
/// OAuth code exchange, and a withings-only section replace of `tokens.json`
/// (ADR-0006, decisions 3–5).
fn run_auth_withings(client: &http::HttpClient, dir: &Path) -> Result<(), AppError> {
    // Load the config if one exists, else start from the skeleton. `auth
    // withings` tolerates empty credentials because it prompts for them next.
    let config_path = config::config_path(dir);
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
        config.withings.client_id = prompt_line(
            "Withings client id (from the Withings developer portal): ",
            "auth withings",
        )?;
    }
    if config.withings.client_secret.trim().is_empty() {
        config.withings.client_secret = prompt_line(
            "Withings client secret (from the Withings developer portal): ",
            "auth withings",
        )?;
    }
    config::write_config(&config_path, &config)?;

    let state = withings::generate_state();
    let authorize_url = withings::authorize_url(&config.withings.client_id, &state);
    println!("Open this URL in a browser and authorize the app:");
    println!("{authorize_url}");
    println!(
        "(If the browser reports a bad redirect URI, register this URI in the Withings developer portal: {})",
        withings::REDIRECT_URI
    );
    let _ = std::io::stdout().flush();

    let pasted = prompt_line(
        "Paste the redirect URL (or the bare code) your browser was sent to: ",
        "auth withings",
    )?;
    let code = withings::extract_code(&pasted, &state)?;

    let withings_tokens = withings::exchange_code(
        client,
        &config.withings.client_id,
        &config.withings.client_secret,
        &code,
    )?;

    // Read-modify-write: replace only the withings section of the shared
    // token file; the garmin section (if any) is left as loaded.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    update_tokens_section(dir, |tokens| {
        tokens.withings = config::WithingsTokens {
            access_token: withings_tokens.access_token,
            refresh_token: withings_tokens.refresh_token.unwrap_or_default(),
            expires_at: Some(now + withings_tokens.expires_in),
        };
    })?;

    println!("auth: Withings connected (scope: user.metrics).");
    Ok(())
}

/// The Garmin half of `auth`: username/password (+MFA) login and a
/// garmin-only section replace of `tokens.json`. Needs no config at all
/// (ADR-0006, decisions 3–5).
fn run_auth_garmin(client: &http::HttpClient, dir: &Path) -> Result<(), AppError> {
    // One-time interactive Garmin mobile-SSO login (ticket 06).
    let username = prompt_line("Garmin username (email): ", "auth garmin")?;
    let password = prompt_line("Garmin password: ", "auth garmin")?;
    let service_ticket = match garmin::login(client, &username, &password)? {
        garmin::LoginOutcome::Success { service_ticket } => service_ticket,
        garmin::LoginOutcome::MfaRequired { method } => {
            let mut attempts = 0;
            loop {
                attempts += 1;
                if attempts > 3 {
                    return Err(AppError::new(
                        EXIT_AUTH,
                        "too many rejected MFA codes; re-run `auth garmin` to try again",
                    ));
                }
                let code = prompt_line(
                    &format!("Garmin MFA code (sent via {method}): "),
                    "auth garmin",
                )?;
                match garmin::verify_mfa(client, &method, &code)? {
                    garmin::MfaOutcome::Ticket(ticket) => break ticket,
                    garmin::MfaOutcome::InvalidCode => {
                        eprintln!("auth: Garmin rejected that MFA code; try again");
                    }
                }
            }
        }
    };
    let garmin_tokens = garmin::exchange_service_ticket(client, &service_ticket)?;

    // Read-modify-write: replace only the garmin section of the shared token
    // file; the withings section (if any) is left as loaded.
    update_tokens_section(dir, |tokens| {
        tokens.garmin = config::GarminTokens {
            access_token: garmin_tokens.access_token,
            refresh_token: garmin_tokens.refresh_token.unwrap_or_default(),
            client_id: Some(garmin_tokens.client_id),
        };
    })?;

    println!("auth: Garmin connected.");
    Ok(())
}

/// Tokens for one auth run: the existing file when present, else defaults.
/// A malformed existing file is a config error (never silently overwritten).
fn load_tokens_or_default(dir: &Path) -> Result<config::Tokens, AppError> {
    if config::tokens_path(dir).exists() {
        config::load_tokens(dir)
    } else {
        Ok(config::Tokens::default())
    }
}

/// Replace one section of the shared `tokens.json` in place, leaving the
/// other service's section as loaded (ADR-0006, decision 3).
fn update_tokens_section<F>(dir: &Path, replace: F) -> Result<(), AppError>
where
    F: FnOnce(&mut config::Tokens),
{
    let mut tokens = load_tokens_or_default(dir)?;
    replace(&mut tokens);
    config::write_tokens(&config::tokens_path(dir), &tokens)
}

/// Mutable Garmin DI token state for one sync run.
struct GarminTokenState {
    access_token: String,
    refresh_token: String,
    client_id: String,
}

/// Write one Garmin record. A `401` triggers a DI token refresh (persisted)
/// followed by a single retry; a rejected refresh is returned as an auth
/// error (`EXIT_AUTH`) so the run aborts with "re-run `auth garmin`".
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
                            "Garmin token refresh failed: {}; re-run `auth garmin`",
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

/// Read one line of operator input, prompting on stderr so stdout stays
/// report-only. Empty input (closed stdin) is an auth error. `retry` names
/// the command to re-run (e.g. `auth withings` or `auth garmin`).
fn prompt_line(label: &str, retry: &str) -> Result<String, AppError> {
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
            format!("no input provided (stdin closed); re-run `{retry}` to continue"),
        ));
    }
    Ok(trimmed)
}

/// Read the Withings measurement window. A token rejection (HTTP 401/403,
/// or a body `status` of 401) triggers a refresh (persisted) followed by a
/// single retry; a rejected refresh or a second rejection is an auth error
/// (`EXIT_AUTH`) so the run aborts with "re-run `auth withings`" (ADR-0007).
fn withings_read<F, T>(
    client: &http::HttpClient,
    tokens_path: &std::path::Path,
    tokens: &mut config::Tokens,
    client_id: &str,
    client_secret: &str,
    read: F,
) -> Result<T, AppError>
where
    F: Fn(&http::HttpClient, &str) -> Result<T, withings::ReadFailure>,
{
    match read(client, &tokens.withings.access_token) {
        Ok(value) => Ok(value),
        Err(withings::ReadFailure::Unauthorized) => {
            let refreshed = withings::refresh(
                client,
                client_id,
                client_secret,
                &tokens.withings.refresh_token,
            )
            .map_err(|error| {
                AppError::new(
                    EXIT_AUTH,
                    format!(
                        "Withings token refresh failed: {}; re-run `auth withings`",
                        error.message
                    ),
                )
            })?;
            tokens.withings.access_token = refreshed.access_token;
            if let Some(rotate) = refreshed.refresh_token {
                tokens.withings.refresh_token = rotate;
            }
            tokens.withings.expires_at = Some(timefmt::now_epoch() as u64 + refreshed.expires_in);
            config::write_tokens(tokens_path, tokens)?;

            // The single retry with the fresh token. A second consecutive
            // rejection means the token pair is dead: exit 4 (unlike Garmin,
            // whose second 401 is a metric failure; ADR-0007 records the
            // asymmetry).
            match read(client, &tokens.withings.access_token) {
                Ok(value) => Ok(value),
                Err(withings::ReadFailure::Unauthorized) => Err(AppError::new(
                    EXIT_AUTH,
                    "token refresh succeeded but Withings still rejected the token; re-run `auth withings`",
                )),
                Err(withings::ReadFailure::Failed(error)) => Err(error),
            }
        }
        Err(withings::ReadFailure::Failed(error)) => Err(error),
    }
}

fn run_sync(cli_args: SyncArgs) -> Result<i32, AppError> {
    // Resolve the metric scope: `sync weight` / `sync bp` / `sync all`;
    // bare `sync` is `all` (ADR-0005). `args` becomes the chosen flag set.
    let (args, scope) = match cli_args.metric {
        Some(metric) => metric.into_parts(),
        None => (cli_args.options, MetricScope::All),
    };
    let include_weight = scope.includes_weight();
    let include_bp = scope.includes_bp();

    let dir = config::resolve_config_dir(args.config_dir.as_deref())?;

    // Fail fast on missing/invalid config or tokens (exit 3).
    let mut config = config::load_config(&dir)?;
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
                    "Withings token refresh failed: {}; re-run `auth withings`",
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

    // Resolve the sync window per metric (ADR-0008): `--since`/`--until`
    // flags win, then the metric's machine-updated floor (`sync.weight.since`
    // / `sync.bp.since`, read strictly newer), then the built-in rolling
    // last 24 hours (the bootstrap for a metric with no floor yet).
    let now = timefmt::now_epoch();
    let until_flag = args.until.as_deref().map(timefmt::parse_date).transpose()?;
    let until = until_flag.unwrap_or(now);
    let since_flag = args.since.as_deref().map(timefmt::parse_date).transpose()?;
    if let (Some(since), Some(until)) = (since_flag, until_flag) {
        if since > until {
            return Err(AppError::new(
                EXIT_USAGE,
                format!("invalid window: --since {since} is after --until {until}"),
            ));
        }
    }

    let weight_bound = MetricBound::resolve(config.sync.weight.since, since_flag, until_flag, now);
    let bp_bound = MetricBound::resolve(config.sync.bp.since, since_flag, until_flag, now);

    let window_label = if since_flag.is_none() && until_flag.is_none() {
        match scope {
            MetricScope::Weight => floor_label("weight", config.sync.weight.since),
            MetricScope::Bp => floor_label("bp", config.sync.bp.since),
            MetricScope::All
                if config.sync.weight.since.is_none() && config.sync.bp.since.is_none() =>
            {
                "last 24 hours".to_string()
            }
            MetricScope::All => format!(
                "{}, {}",
                floor_label("weight", config.sync.weight.since),
                floor_label("bp", config.sync.bp.since)
            ),
        }
    } else {
        match (since_flag, until_flag) {
            (Some(_), Some(_)) => format!(
                "{}..{}",
                args.since.as_deref().unwrap(),
                args.until.as_deref().unwrap()
            ),
            (Some(_), None) => format!("{}..now", args.since.as_deref().unwrap()),
            (None, Some(_)) => format!("..{}", args.until.as_deref().unwrap()),
            (None, None) => "last 24 hours".to_string(),
        }
    };

    // Read the window from Withings once, scoped to the metric(s) in play
    // (ADR-0005): a single-metric run never fetches the other metric. The
    // combined run reads at the earliest per-metric bound and filters each
    // metric client-side to its own bound (ADR-0008). A mid-read auth
    // rejection refreshes the token once and retries (ADR-0007).
    let read_start = match (include_weight, include_bp) {
        (true, true) => weight_bound.epoch.min(bp_bound.epoch),
        (true, false) => weight_bound.epoch,
        (false, true) => bp_bound.epoch,
        (false, false) => unreachable!("a sync scope always includes at least one metric"),
    };
    // A bound newer than the upper bound (e.g. a floor slightly in the
    // future after clock skew) reads nothing rather than sending Withings
    // an inverted window.
    let mut groups = Vec::new();
    let tokens_path = config::tokens_path(&dir);
    if read_start <= until {
        groups = withings_read(
            &client,
            &tokens_path,
            &mut tokens,
            &config.withings.client_id,
            &config.withings.client_secret,
            |c, t| withings::read_measures(c, t, read_start, until, scope.meastypes()),
        )?;
    }
    let (weights, bps, skips) = transform::transform(groups);

    // Each metric keeps only readings at or after its own resolved bound. A
    // floor bound is always filtered (strictly newer: `floor + 1`), so the
    // last-written measurement is never re-processed; a rolling bound is
    // filtered only when the combined read started earlier than it.
    let weight_keep_from = if include_weight {
        weight_bound
            .keep_from
            .or_else(|| (weight_bound.epoch > read_start).then_some(weight_bound.epoch))
    } else {
        None
    };
    let bp_keep_from = if include_bp {
        bp_bound
            .keep_from
            .or_else(|| (bp_bound.epoch > read_start).then_some(bp_bound.epoch))
    } else {
        None
    };
    let weights: Vec<transform::WeightReading> = weights
        .into_iter()
        .filter(|w| weight_keep_from.is_none_or(|bound| w.epoch >= bound))
        .collect();
    let bps: Vec<transform::BpReading> = bps
        .into_iter()
        .filter(|b| bp_keep_from.is_none_or(|bound| b.epoch >= bound))
        .collect();
    let skips: Vec<transform::Skip> = skips
        .into_iter()
        .filter(|skip| match skip.metric {
            transform::Metric::Weight => weight_keep_from.is_none_or(|bound| skip.epoch >= bound),
            transform::Metric::BloodPressure => {
                bp_keep_from.is_none_or(|bound| skip.epoch >= bound)
            }
        })
        .collect();

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
        match scope {
            MetricScope::All => println!(
                "dry-run ({window_label}): would write {} weight and {} blood-pressure measurements",
                weights.len(),
                bps.len()
            ),
            MetricScope::Weight => println!(
                "dry-run ({window_label}): would write {} weight measurements",
                weights.len()
            ),
            MetricScope::Bp => println!(
                "dry-run ({window_label}): would write {} blood-pressure measurements",
                bps.len()
            ),
        }
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
        // `summary:` names the metric(s) in play (ADR-0005, decision 6).
        match scope {
            MetricScope::All => println!("summary: dry-run complete"),
            MetricScope::Weight => println!("summary: weight dry-run complete"),
            MetricScope::Bp => println!("summary: blood-pressure dry-run complete"),
        }
        return Ok(EXIT_OK);
    }

    // Apply: write each metric independently; one failing does not block the
    // other. A 401 refreshes the Garmin DI token once and retries.
    // Weight has no skip path today (transform either emits a reading or
    // stays silent on it), so this count is 0; it keeps the weight report
    // line's "skipped" slot explicit rather than magic.
    let weight_skips = skips
        .iter()
        .filter(|s| s.metric == transform::Metric::Weight)
        .count();
    let bp_skips = skips
        .iter()
        .filter(|s| s.metric == transform::Metric::BloodPressure)
        .count();

    let mut garmin_state = GarminTokenState {
        access_token: tokens.garmin.access_token.clone(),
        refresh_token: tokens.garmin.refresh_token.clone(),
        client_id: tokens.garmin.client_id.clone().unwrap_or_default(),
    };

    let mut weight_failed = 0usize;
    let mut bp_failed = 0usize;
    // The newest Withings timestamp among each metric's successfully written
    // measurements; the floors advance to these after a clean metric.
    let mut weight_newest: Option<i64> = None;
    let mut bp_newest: Option<i64> = None;
    if include_weight {
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
                Ok(()) => {
                    weight_newest =
                        Some(weight_newest.map_or(weight.epoch, |m| m.max(weight.epoch)));
                }
                Err(error) if error.code == EXIT_AUTH => return Err(error),
                Err(error) => {
                    eprintln!("error: weight write failed: {}", error.message);
                    weight_failed += 1;
                }
            }
        }
    }
    if include_bp {
        for bp in &bps {
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
                Ok(()) => {
                    bp_newest = Some(bp_newest.map_or(bp.epoch, |m| m.max(bp.epoch)));
                }
                Err(error) if error.code == EXIT_AUTH => return Err(error),
                Err(error) => {
                    eprintln!("error: blood-pressure write failed: {}", error.message);
                    bp_failed += 1;
                }
            }
        }
    }

    let weight_written = weights.len().saturating_sub(weight_failed);
    let bp_written = bps.len().saturating_sub(bp_failed);
    let any_failed = weight_failed > 0 || bp_failed > 0;

    // Floor advancement (ADR-0008): a metric's floor moves to the newest
    // written timestamp only when every one of that metric's writes
    // succeeded; empty runs and failed metrics leave the floor untouched.
    // The whole config is rewritten through the existing 0600 write path. A
    // config-write failure after successful Garmin writes warns but does not
    // change the run's reported outcome.
    let mut floors_changed = false;
    if include_weight && weight_failed == 0 {
        if let Some(epoch) = weight_newest {
            config.sync.weight.since = Some(epoch);
            floors_changed = true;
        }
    }
    if include_bp && bp_failed == 0 {
        if let Some(epoch) = bp_newest {
            config.sync.bp.since = Some(epoch);
            floors_changed = true;
        }
    }
    if floors_changed {
        if let Err(error) = config::write_config(&config::config_path(&dir), &config) {
            eprintln!(
                "warning: could not write config after Garmin writes succeeded: {}; \
                 the sync floors did not advance and the next run may re-send measurements",
                error.message
            );
        }
    }

    // Reporting is metric-aware (ADR-0005): a single-metric run reports only
    // its own metric and names it in the summary. Exit codes are unchanged.
    match scope {
        MetricScope::All => {
            println!(
                "apply ({window_label}): weight: {weight_written} written, {weight_skips} skipped, {weight_failed} failed"
            );
            println!(
                "apply ({window_label}): blood-pressure: {bp_written} written, {bp_skips} skipped, {bp_failed} failed"
            );
            if any_failed {
                println!("summary: 1 metric(s) failed");
                Ok(EXIT_METRIC_FAILURE)
            } else {
                println!("summary: all metrics synced");
                Ok(EXIT_OK)
            }
        }
        MetricScope::Weight => {
            println!(
                "apply ({window_label}): weight: {weight_written} written, {weight_skips} skipped, {weight_failed} failed"
            );
            if any_failed {
                println!("summary: weight failed");
                Ok(EXIT_METRIC_FAILURE)
            } else {
                println!("summary: weight synced");
                Ok(EXIT_OK)
            }
        }
        MetricScope::Bp => {
            println!(
                "apply ({window_label}): blood-pressure: {bp_written} written, {bp_skips} skipped, {bp_failed} failed"
            );
            if any_failed {
                println!("summary: blood-pressure failed");
                Ok(EXIT_METRIC_FAILURE)
            } else {
                println!("summary: blood-pressure synced");
                Ok(EXIT_OK)
            }
        }
    }
}
