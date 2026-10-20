pub mod cli;
pub mod config;
pub mod garmin;
pub mod http;
pub mod metric;
pub mod session;
pub mod timefmt;
pub mod transform;
pub mod withings;

use std::fs;
use std::io::{IsTerminal, Write};
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
pub use metric::Metric;

#[derive(Debug)]
pub enum Fault {
    Unauthorized,
    Failed(AppError),
}

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
    /// Metrics included in this run, in stable report and query order.
    fn metrics(self) -> Vec<Metric> {
        match self {
            MetricScope::Weight => vec![Metric::Weight],
            MetricScope::Bp => vec![Metric::BloodPressure],
            MetricScope::All => Metric::ALL.to_vec(),
        }
    }

    /// The Withings meastype set this scope requests.
    fn meastypes(self) -> String {
        self.metrics()
            .iter()
            .map(|metric| metric.meastypes())
            .collect::<Vec<_>>()
            .join(",")
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

/// The flag-free report label for one metric's bound: its floor in the
/// canonical RFC 3339 UTC form (ADR-0009), or the rolling bootstrap.
fn floor_label(name: &str, floor: Option<i64>) -> String {
    match floor {
        Some(floor) => format!("{name} floor {}", timefmt::format_floor(floor)),
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
    let mut config =
        config::read_config_file(&config_path)?.unwrap_or_else(config::Config::skeleton);

    // One-time interactive Withings OAuth (ticket 05).
    if config.withings.client_id.trim().is_empty() {
        config.withings.client_id = prompt_line(
            "Withings client id (from the Withings developer portal): ",
            "auth withings",
        )?;
    }
    if config.withings.client_secret.trim().is_empty() {
        config.withings.client_secret = prompt_secret(
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
    let password = prompt_secret("Garmin password: ", "auth garmin")?;
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
                let code = prompt_secret(
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

/// Replace one section of the shared token document in place (ADR-0006).
fn update_tokens_section<F>(dir: &Path, replace: F) -> Result<(), AppError>
where
    F: FnOnce(&mut config::Tokens),
{
    let mut session = session::Session::load_or_default(dir)?;
    session.replace(replace);
    session.persist()
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

/// Read a secret without exposing it on a terminal. Piped input stays
/// scriptable and is explicitly reported as unmasked on stderr.
fn prompt_secret(label: &str, retry: &str) -> Result<String, AppError> {
    let input = std::io::stdin();
    if input.is_terminal() {
        let config = rpassword::ConfigBuilder::new()
            .password_feedback_mask('*')
            .build();
        let secret = rpassword::prompt_password_with_config(label, config)
            .map_err(|error| AppError::new(EXIT_AUTH, format!("could not read input: {error}")))?;
        return validate_prompt_input(secret, retry);
    }

    eprint!("{label}");
    eprintln!("\nauth: stdin is not a terminal; input is being read unmasked");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|error| AppError::new(EXIT_AUTH, format!("could not read input: {error}")))?;
    validate_prompt_input(line, retry)
}

fn validate_prompt_input(line: String, retry: &str) -> Result<String, AppError> {
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::new(
            EXIT_AUTH,
            format!("no input provided (stdin closed); re-run `{retry}` to continue"),
        ));
    }
    Ok(trimmed)
}

fn run_sync(cli_args: SyncArgs) -> Result<i32, AppError> {
    // Resolve the metric scope: `sync weight` / `sync bp` / `sync all`;
    // bare `sync` is `all` (ADR-0005). `args` becomes the chosen flag set.
    let (args, scope) = match cli_args.metric {
        Some(metric) => metric.into_parts(),
        None => (cli_args.options, MetricScope::All),
    };
    let metrics = scope.metrics();
    let include_weight = metrics.contains(&Metric::Weight);
    let include_bp = metrics.contains(&Metric::BloodPressure);

    let dir = config::resolve_config_dir(args.config_dir.as_deref())?;

    // Fail fast on missing/invalid config or tokens (exit 3).
    let mut config = config::load_config(&dir)?;
    let mut session = session::Session::load(&dir)?;
    let tokens = session.tokens();
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
    session.refresh_withings_if_expiring(
        &client,
        &config.withings.client_id,
        &config.withings.client_secret,
    )?;

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

    // A stored floor ahead of the local clock never advances: the monotonic
    // guard (ADR-0010) refuses to lower it, so that metric's strictly-newer
    // reads see nothing forever and every run reports 0 written / 0 skipped
    // / 0 failed while exiting clean. The only remedy is a hand-lowered
    // floor, so warn up front — scoped to the metrics this run touches
    // (ADR-0005) — instead of failing silently.
    for (name, key, floor, included) in [
        (
            "weight",
            "sync.weight.since",
            config.sync.weight.since,
            include_weight,
        ),
        ("bp", "sync.bp.since", config.sync.bp.since, include_bp),
    ] {
        if !included {
            continue;
        }
        let Some(floor) = floor else {
            continue;
        };
        if floor > now {
            eprintln!(
                "warning: {name} floor {} is in the future; sync will not advance it — \
                 hand-lower {key} in config.toml to resume {name} sync",
                timefmt::format_floor(floor)
            );
        }
    }

    let weight_bound = MetricBound::resolve(
        config.sync.floor(Metric::Weight),
        since_flag,
        until_flag,
        now,
    );
    let bp_bound = MetricBound::resolve(
        config.sync.floor(Metric::BloodPressure),
        since_flag,
        until_flag,
        now,
    );

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
    // an inverted window; with no request fired there is no query
    // timestamp, so nothing can advance (ADR-0010).
    let mut groups = Vec::new();
    let mut query_ts: Option<i64> = None;
    if read_start <= until {
        let read = session.recover_withings(
            &client,
            &config.withings.client_id,
            &config.withings.client_secret,
            |t| withings::read_measures(&client, t, read_start, until_flag, &scope.meastypes()),
        )?;
        groups = read.groups;
        query_ts = Some(read.query_ts);
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
            Metric::Weight => weight_keep_from.is_none_or(|bound| skip.epoch >= bound),
            Metric::BloodPressure => bp_keep_from.is_none_or(|bound| skip.epoch >= bound),
        })
        .collect();

    for skip in &skips {
        eprintln!(
            "warning: skipping {} reading at {}: {}",
            match skip.metric {
                Metric::Weight => "weight",
                Metric::BloodPressure => "blood-pressure",
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
    let weight_skips = skips.iter().filter(|s| s.metric == Metric::Weight).count();
    let bp_skips = skips
        .iter()
        .filter(|s| s.metric == Metric::BloodPressure)
        .count();

    let mut weight_failed = 0usize;
    let mut bp_failed = 0usize;
    if include_weight {
        for weight in &weights {
            let payload = garmin::weight_payload(
                &timefmt::local_ms(weight.epoch),
                &timefmt::gmt_ms(weight.epoch),
                weight.kg,
            );
            match session.recover_garmin(&client, |t| garmin::write_weight(&client, t, &payload)) {
                Ok(()) => {}
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
            match session.recover_garmin(&client, |t| {
                garmin::write_blood_pressure(&client, t, &payload)
            }) {
                Ok(()) => {}
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

    // Floor advancement (ADR-0010, amending ADR-0008): on a clean flag-free
    // apply, each included metric's floor advances to the Withings query
    // timestamp — the enddate actually sent, captured at the moment the
    // request fired — with a monotonic guard (never lower than the current
    // floor). Failed writes still block that metric (per-metric
    // all-or-nothing); skips are not failures. A floorless metric gains a
    // floor on its first clean flag-free apply. Flag-driven applies and
    // inverted windows (no request fired) advance nothing. The whole config
    // is rewritten through the existing 0600 write path; a config-write
    // failure after the apply warns and leaves the floors unadvanced
    // without changing the run's reported outcome.
    let flag_free = since_flag.is_none() && until_flag.is_none();
    let mut floors_moved: Vec<(&'static str, i64)> = Vec::new();
    if flag_free {
        if let Some(query_ts) = query_ts {
            for (metric, failed) in [
                (Metric::Weight, weight_failed),
                (Metric::BloodPressure, bp_failed),
            ] {
                if !scope.metrics().contains(&metric) || failed > 0 {
                    continue;
                }
                if config.sync.advance_floor(metric, query_ts) {
                    floors_moved.push((
                        if metric == Metric::Weight {
                            "weight"
                        } else {
                            "bp"
                        },
                        query_ts,
                    ));
                }
            }
        }
    }
    if !floors_moved.is_empty() {
        if let Err(error) = config::write_config(&config::config_path(&dir), &config) {
            eprintln!(
                "warning: could not write config after the apply: {}; \
                 the sync floors did not advance and the next run may re-read this window",
                error.message
            );
            floors_moved.clear();
        }
    }
    // The report notes an advancement per metric, only when that metric's
    // floor actually moved and was persisted (ADR-0010).
    let weight_note = floors_moved
        .iter()
        .find(|(name, _)| *name == "weight")
        .map(|(_, ts)| format!(" — floor advanced to {}", timefmt::format_floor(*ts)));
    let bp_note = floors_moved
        .iter()
        .find(|(name, _)| *name == "bp")
        .map(|(_, ts)| format!(" — floor advanced to {}", timefmt::format_floor(*ts)));

    // Reporting is metric-aware (ADR-0005): a single-metric run reports only
    // its own metric and names it in the summary. Exit codes are unchanged.
    match scope {
        MetricScope::All => {
            println!(
                "apply ({window_label}): weight: {weight_written} written, {weight_skips} skipped, {weight_failed} failed{}",
                weight_note.as_deref().unwrap_or("")
            );
            println!(
                "apply ({window_label}): blood-pressure: {bp_written} written, {bp_skips} skipped, {bp_failed} failed{}",
                bp_note.as_deref().unwrap_or("")
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
                "apply ({window_label}): weight: {weight_written} written, {weight_skips} skipped, {weight_failed} failed{}",
                weight_note.as_deref().unwrap_or("")
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
                "apply ({window_label}): blood-pressure: {bp_written} written, {bp_skips} skipped, {bp_failed} failed{}",
                bp_note.as_deref().unwrap_or("")
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
