//! Black-box tests for ADR-0008/ADR-0009: per-metric machine-updated sync
//! floors stored as human-readable RFC 3339 UTC datetimes.
//!
//! `sync.weight.since` / `sync.bp.since` in config.toml mark the newest
//! Withings measurement already written to Garmin. A successful apply
//! advances each metric's floor independently; the next run reads strictly
//! newer data (`startdate = floor + 1`) and the day-granular Garmin BP
//! read-back no longer exists. Driven through the binary against fake
//! Withings and Garmin servers, asserting on requests received, exit codes,
//! output, and the files written.

mod common;

use common::{
    file_mode, form_value, parse_form, run_bin, run_bin_stdin, write_file,
    write_valid_config_and_tokens, FakeResponse, FakeServer, Route, TempDir, VALID_TOKENS,
};
use serde_json::json;

/// 2026-01-02T08:30:00Z — a Withings measurement timestamp.
const EPOCH: i64 = 1767342600;
/// 2026-01-01T00:00:00Z.
const JAN1: i64 = 1767225600;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// A Withings `getmeas` response page.
fn measures_page(groups: serde_json::Value, more: u8, offset: u32) -> String {
    json!({
        "status": 0,
        "body": {
            "updatetime": EPOCH,
            "timezone": "UTC",
            "measuregrps": groups,
            "more": more,
            "offset": offset
        }
    })
    .to_string()
}

fn weight_group(value: i64, unit: i16, date: i64) -> serde_json::Value {
    json!({
        "grpid": 1,
        "attrib": 2,
        "date": date,
        "category": 1,
        "measures": [{"value": value, "type": 1, "unit": unit}]
    })
}

fn bp_group(systolic: i64, diastolic: i64, pulse: Option<i64>, date: i64) -> serde_json::Value {
    let mut measures = vec![
        json!({"value": systolic, "type": 10, "unit": 0}),
        json!({"value": diastolic, "type": 9, "unit": 0}),
    ];
    if let Some(pulse) = pulse {
        measures.push(json!({"value": pulse, "type": 11, "unit": 0}));
    }
    json!({"grpid": 2, "attrib": 2, "date": date, "category": 1, "measures": measures})
}

fn single_page_withings(groups: Vec<serde_json::Value>) -> FakeServer {
    let groups = serde_json::Value::Array(groups);
    FakeServer::start(vec![Route::post("/measure", move |_req, _i| {
        FakeResponse::json(200, &measures_page(groups.clone(), 0, 0))
    })])
}

/// Garmin API fake that accepts weight (204) and BP (200) writes. The BP
/// range read-back route is deliberately absent (ADR-0008 deleted it): any
/// run that still calls it would get a 404, and the tests assert no GET
/// request ever reaches Garmin.
fn ok_garmin() -> FakeServer {
    FakeServer::start(vec![
        Route::post("/weight-service/user-weight", |_req, _i| {
            FakeResponse::new(204, "")
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, _i| {
            FakeResponse::json(200, "{}")
        }),
    ])
}

fn sync_env<'a>(withings: &'a FakeServer, garmin: &'a FakeServer) -> Vec<(&'static str, &'a str)> {
    vec![
        ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
        ("WGS_GARMIN_API_BASE", garmin.base_url.as_str()),
        ("TZ", "UTC"),
    ]
}

fn run_sync(
    dir: &TempDir,
    args: &[&str],
    withings: &FakeServer,
    garmin: &FakeServer,
) -> common::Run {
    let mut full: Vec<&str> = vec!["sync"];
    let rest: &[&str] = match args.first() {
        Some(&"weight" | &"bp" | &"all") => {
            full.push(args[0]);
            &args[1..]
        }
        _ => args,
    };
    full.push("--config-dir");
    full.push(dir.path().to_str().unwrap());
    full.extend_from_slice(rest);
    run_bin(&full, &sync_env(withings, garmin))
}

/// A config.toml with the given credentials and per-metric floors (absent
/// floors serialize to nothing). Floors are written in the canonical
/// RFC 3339 UTC form, exactly as the machine writes them (ADR-0009).
fn floored_config(
    weight: Option<i64>,
    bp: Option<i64>,
    client_id: &str,
    client_secret: &str,
) -> String {
    let mut text =
        format!("[withings]\nclient_id = \"{client_id}\"\nclient_secret = \"{client_secret}\"\n");
    if let Some(floor) = weight {
        text.push_str(&format!("\n[sync.weight]\nsince = \"{}\"\n", floor_iso(floor)));
    }
    if let Some(floor) = bp {
        text.push_str(&format!("\n[sync.bp]\nsince = \"{}\"\n", floor_iso(floor)));
    }
    text
}

/// The canonical RFC 3339 UTC floor form for an epoch (the machine-written
/// format; ADR-0009).
fn floor_iso(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .expect("test epochs are in range")
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn read_config(dir: &TempDir) -> toml::Value {
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    toml::from_str(&text).unwrap()
}

fn config_text(dir: &TempDir) -> String {
    std::fs::read_to_string(dir.path().join("config.toml")).unwrap()
}

/// `sync.<metric>.since` as an epoch-second integer, or `None` when the
/// metric has no floor (or no `[sync]` section at all). Accepts the
/// canonical RFC 3339 string the machine writes and legacy integers (for
/// reading a config before its next rewrite).
fn floor(dir: &TempDir, metric: &str) -> Option<i64> {
    let config = read_config(dir);
    let since = config
        .get("sync")
        .and_then(|sync| sync.get(metric))
        .and_then(|metric| metric.get("since"))?;
    match since {
        toml::Value::Integer(epoch) => Some(*epoch),
        toml::Value::String(text) => Some(
            chrono::DateTime::parse_from_rfc3339(text)
                .expect("the machine writes floors in canonical RFC 3339")
                .timestamp(),
        ),
        other => panic!("unexpected floor value: {other:?}"),
    }
}

fn startdate(read: &common::RecordedRequest) -> i64 {
    let fields = parse_form(&read.body);
    form_value(&fields, "startdate").parse().unwrap()
}

// ---------------------------------------------------------------------------
// Weight floor (ticket 01)
// ---------------------------------------------------------------------------

#[test]
fn weight_apply_with_no_floor_bootstraps_and_writes_floor() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        weight_group(82500, -3, EPOCH + 120),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["weight", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // First run with no floor: the rolling last-24-hours startdate.
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1, "{:#?}", withings.requests());
    let n = now();
    assert!(
        (n - 86400 - startdate(&reads[0])).abs() < 120,
        "startdate {} not ~1d ago",
        startdate(&reads[0])
    );

    // Both measurements written, and the floor lands on the newest one.
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        2,
        "{:#?}",
        garmin.requests()
    );
    assert_eq!(floor(&dir, "weight"), Some(EPOCH + 120));
    assert_eq!(floor(&dir, "bp"), None);
    // The rewritten config keeps the 0600 permissions (user story 23).
    assert_eq!(file_mode(&dir.path().join("config.toml")), 0o600);
}

#[test]
fn weight_apply_rerun_reads_strictly_newer_and_writes_nothing() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH + 120),
            None,
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    // Unchanged Withings data: the newest measurement sits exactly at the
    // stored floor and must never be re-read or re-written.
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        weight_group(82500, -3, EPOCH + 120),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["weight", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1);
    // Strictly newer than the floor: startdate = floor + 1.
    assert_eq!(startdate(&reads[0]), EPOCH + 121);
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());
    assert!(
        run.stdout
            .contains("weight: 0 written, 0 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
    // The floor is untouched.
    assert_eq!(floor(&dir, "weight"), Some(EPOCH + 120));
}

#[test]
fn dry_run_never_writes_config() {
    // With floors already present: a dry run must leave the file byte-identical.
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH),
            Some(EPOCH + 1),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![weight_group(82400, -3, EPOCH + 60)]);
    let garmin = ok_garmin();
    let before = config_text(&dir);

    let run = run_sync(&dir, &["weight"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(config_text(&dir), before);

    // Without any floor: a dry run must not create a [sync] section either.
    let dir2 = TempDir::new();
    write_valid_config_and_tokens(dir2.path());
    let before2 = config_text(&dir2);
    let run2 = run_sync(&dir2, &[], &withings, &garmin);
    assert_eq!(run2.code, 0, "stderr: {}", run2.stderr);
    assert_eq!(config_text(&dir2), before2);
}

#[test]
fn empty_apply_never_writes_config() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![]);
    let garmin = ok_garmin();
    let before = config_text(&dir);

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());
    assert_eq!(config_text(&dir), before);
}

#[test]
fn failed_weight_writes_leave_floor_untouched() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(Some(EPOCH), None, "test-client-id", "test-client-secret"),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![weight_group(82500, -3, EPOCH + 120)]);
    let garmin = FakeServer::start(vec![Route::post(
        "/weight-service/user-weight",
        |_req, _i| FakeResponse::new(500, "boom"),
    )]);

    let run = run_sync(&dir, &["weight", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 1, "stdout: {}", run.stdout);
    assert!(
        run.stdout
            .contains("weight: 0 written, 0 skipped, 1 failed"),
        "stdout: {}",
        run.stdout
    );
    // A failed metric never advances its floor.
    assert_eq!(floor(&dir, "weight"), Some(EPOCH));
}

#[test]
fn legacy_shared_sync_since_still_loads_and_is_ignored() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n\n[sync]\nsince = \"2026-01-15\"\n",
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &[], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1);
    // The legacy key is dropped from the schema: the rolling window wins.
    let n = now();
    assert!(
        (n - 86400 - startdate(&reads[0])).abs() < 120,
        "startdate {} not ~1d ago",
        startdate(&reads[0])
    );
}

#[test]
fn sync_all_filters_each_metric_to_its_own_floor_in_one_read() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH),
            Some(EPOCH + 50),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),          // == weight floor: dropped
        weight_group(82500, -3, EPOCH + 60),     // written
        bp_group(120, 80, Some(72), EPOCH + 40), // <= bp floor: dropped
        bp_group(121, 81, None, EPOCH + 90),     // written
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["all", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // One Withings round-trip for the combined run, at the earliest bound
    // (weight floor + 1), with the combined meastype set.
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1, "{:#?}", withings.requests());
    let fields = parse_form(&reads[0].body);
    assert_eq!(form_value(&fields, "startdate"), (EPOCH + 1).to_string());
    assert_eq!(form_value(&fields, "meastypes"), "1,9,10,11");

    // Each metric wrote only what is strictly newer than its own floor.
    let weight_calls = garmin.requests_for("POST", "/weight-service/user-weight");
    assert_eq!(weight_calls.len(), 1, "{:#?}", garmin.requests());
    let payload: serde_json::Value = serde_json::from_str(&weight_calls[0].body).unwrap();
    assert_eq!(payload["value"], 82.5);
    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    assert_eq!(bp_calls.len(), 1, "{:#?}", garmin.requests());
    let payload: serde_json::Value = serde_json::from_str(&bp_calls[0].body).unwrap();
    assert_eq!(payload["systolic"], 121);

    // Both floors advanced independently to their newest written timestamp.
    assert_eq!(floor(&dir, "weight"), Some(EPOCH + 60));
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 90));
}

#[test]
fn sync_all_bp_bootstraps_while_weight_uses_its_floor() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(Some(EPOCH), None, "test-client-id", "test-client-secret"),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let recent_bp = now() - 60;
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),      // == weight floor: dropped
        weight_group(82500, -3, EPOCH + 60), // written
        // No bp floor: the rolling 24-hour bootstrap, so only a recent
        // reading falls inside BP's own window.
        bp_group(120, 80, Some(72), recent_bp),
        bp_group(121, 81, None, EPOCH + 1), // older than 24h: outside BP's window
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["all", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1, "{:#?}", withings.requests());
    assert_eq!(startdate(&reads[0]), EPOCH + 1);
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        1
    );
    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    assert_eq!(bp_calls.len(), 1, "{:#?}", garmin.requests());
    let payload: serde_json::Value = serde_json::from_str(&bp_calls[0].body).unwrap();
    assert_eq!(payload["systolic"], 120);
    // The bootstrap write gives BP its first floor.
    assert_eq!(floor(&dir, "weight"), Some(EPOCH + 60));
    assert_eq!(floor(&dir, "bp"), Some(recent_bp));
}

#[test]
fn auth_withings_preserves_floors_when_rewriting_config() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(Some(EPOCH), Some(EPOCH + 1), "", ""),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = FakeServer::start(vec![Route::post("/v2/oauth2", |_req, _i| {
        FakeResponse::json(
            200,
            r#"{"status":0,"body":{"userid":42,"access_token":"new-wa","refresh_token":"new-wr","expires_in":10800,"scope":"user.metrics","token_type":"Bearer"}}"#,
        )
    })]);

    let run = run_bin_stdin(
        &[
            "auth",
            "withings",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", "http://127.0.0.1:1"),
            ("WGS_GARMIN_DIAUTH_BASE", "http://127.0.0.1:1"),
            ("WGS_GARMIN_API_BASE", "http://127.0.0.1:1"),
        ],
        Some("my-client-id\nmy-client-secret\nhttp://localhost:8765/?code=abc123\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let config = read_config(&dir);
    assert_eq!(
        config["withings"]["client_id"].as_str(),
        Some("my-client-id")
    );
    assert_eq!(floor(&dir, "weight"), Some(EPOCH));
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 1));
    // The auth rewrite preserves the floors and emits them canonically.
    let text = config_text(&dir);
    assert!(
        text.contains("since = \"2026-01-02T08:30:00Z\""),
        "text: {text}"
    );
    assert!(
        text.contains("since = \"2026-01-02T08:30:01Z\""),
        "text: {text}"
    );
}

#[test]
fn auth_garmin_leaves_floored_config_untouched() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH),
            Some(EPOCH + 1),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let before = config_text(&dir);
    let sso = FakeServer::start(vec![
        Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
            FakeResponse::new(200, "ok")
        }),
        Route::post("/mobile/api/login", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": "ST-1"}"#,
            )
        }),
    ]);
    let diauth = FakeServer::start(vec![Route::post(
        "/di-oauth2-service/oauth/token",
        |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"access_token": "new-ga", "refresh_token": "new-gr"}"#,
            )
        },
    )]);

    let run = run_bin_stdin(
        &[
            "auth",
            "garmin",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ],
        &[
            ("WGS_WITHINGS_API_BASE", "http://127.0.0.1:1"),
            ("WGS_GARMIN_SSO_BASE", sso.base_url.as_str()),
            ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ],
        Some("user@example.com\nsecret\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(config_text(&dir), before);
}

// ---------------------------------------------------------------------------
// RFC 3339 floors, hand-edits, and migration (ADR-0009, ticket 04)
// ---------------------------------------------------------------------------

#[test]
fn legacy_integer_floors_load_and_are_rewritten_canonically_by_the_next_apply() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n\n\
         [sync.weight]\nsince = 1767342600\n\n[sync.bp]\nsince = 1767342601\n",
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![
        weight_group(82500, -3, EPOCH + 120),
        bp_group(120, 80, Some(72), EPOCH + 90),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["all", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    // The legacy integers load with their exact meaning: strictly newer.
    assert_eq!(
        startdate(&withings.requests_for("POST", "/measure")[0]),
        EPOCH + 1
    );
    // Both floors advanced, and the rewrite emitted the canonical RFC 3339
    // form (no integers anywhere).
    assert_eq!(floor(&dir, "weight"), Some(EPOCH + 120));
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 90));
    let text = config_text(&dir);
    assert!(!text.contains("since = 1767"), "text: {text}");
    assert!(
        text.contains("since = \"2026-01-02T08:32:00Z\""),
        "text: {text}"
    );
    assert!(
        text.contains("since = \"2026-01-02T08:31:30Z\""),
        "text: {text}"
    );
}

#[test]
fn hand_written_date_floor_loads_as_midnight_utc_and_backfills() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n\n\
         [sync.bp]\nsince = \"2026-01-01\"\n",
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![bp_group(120, 80, Some(72), EPOCH)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    // The date means midnight UTC (same as --since): startdate = JAN1 + 1,
    // so the EPOCH measurement is inside the window and gets written.
    assert_eq!(
        startdate(&withings.requests_for("POST", "/measure")[0]),
        JAN1 + 1
    );
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        1
    );
    // The advance is written in canonical form.
    assert_eq!(floor(&dir, "bp"), Some(EPOCH));
    let text = config_text(&dir);
    assert!(
        text.contains("since = \"2026-01-02T08:30:00Z\""),
        "text: {text}"
    );
}

#[test]
fn hand_edited_offset_and_fractional_floors_load_and_normalize() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n\n\
         [sync.weight]\nsince = \"2026-01-02T10:30:00+02:00\"\n\n\
         [sync.bp]\nsince = \"2026-01-02T08:30:00.999Z\"\n",
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![
        weight_group(82500, -3, EPOCH + 120),
        bp_group(120, 80, Some(72), EPOCH + 90),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["all", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    // Both forms load as the EPOCH instant: the offset converts to UTC and
    // the fraction truncates toward the floor (never up), so the read stays
    // strictly newer: startdate = EPOCH + 1.
    assert_eq!(
        startdate(&withings.requests_for("POST", "/measure")[0]),
        EPOCH + 1
    );
    assert_eq!(floor(&dir, "weight"), Some(EPOCH + 120));
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 90));
    // The rewrite normalized both hand-edits to the canonical form.
    let text = config_text(&dir);
    assert!(
        text.contains("since = \"2026-01-02T08:32:00Z\""),
        "text: {text}"
    );
    assert!(
        text.contains("since = \"2026-01-02T08:31:30Z\""),
        "text: {text}"
    );
    assert!(!text.contains("+02:00"), "text: {text}");
    assert!(!text.contains(".999"), "text: {text}");
}

#[test]
fn malformed_floors_fail_config_load_with_exit_3() {
    for (metric, value) in [
        ("weight", "junk"),
        ("weight", "2026-01-02T08:30:00"),      // naive: no timezone
        ("bp", "2026-01-02 08:30:00Z"),         // space separator
        ("bp", "2026-13-45"),                   // invalid date
    ] {
        let dir = TempDir::new();
        write_file(
            &dir.path().join("config.toml"),
            &format!(
                "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n\n\
                 [sync.{metric}]\nsince = {value:?}\n"
            ),
        );
        let withings = single_page_withings(vec![]);
        let garmin = ok_garmin();

        let run = run_sync(&dir, &[], &withings, &garmin);

        assert_eq!(run.code, 3, "stdout: {}", run.stdout);
        assert!(
            run.stderr.contains("invalid config"),
            "stderr for {metric}/{value:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains(&format!("sync.{metric}.since")),
            "stderr for {metric}/{value:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("RFC 3339"),
            "stderr for {metric}/{value:?}: {}",
            run.stderr
        );
        // Nothing ran: config load fails before any network read.
        assert!(withings.requests().is_empty(), "{:#?}", withings.requests());
    }
}

#[test]
fn flag_free_report_labels_show_iso_floors() {
    // Combined flag-free run labels both floors in the canonical form.
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH),
            Some(EPOCH + 1),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &[], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains(
            "dry-run (weight floor 2026-01-02T08:30:00Z, bp floor 2026-01-02T08:30:01Z): \
             would write 0 weight and 0 blood-pressure measurements"
        ),
        "stdout: {}",
        run.stdout
    );

    // Single-metric label.
    let run = run_sync(&dir, &["weight"], &withings, &garmin);
    assert!(
        run.stdout
            .contains("dry-run (weight floor 2026-01-02T08:30:00Z)"),
        "stdout: {}",
        run.stdout
    );

    // The rolling bootstrap label is unchanged.
    let dir2 = TempDir::new();
    write_valid_config_and_tokens(dir2.path());
    let run2 = run_sync(&dir2, &[], &withings, &garmin);
    assert!(
        run2.stdout.contains("dry-run (last 24 hours)"),
        "stdout: {}",
        run2.stdout
    );

    // Flag-derived labels are untouched.
    let run3 = run_sync(
        &dir2,
        &["--since", "2026-01-01", "--until", "2026-02-01"],
        &withings,
        &garmin,
    );
    assert!(
        run3.stdout.contains("dry-run (2026-01-01..2026-02-01)"),
        "stdout: {}",
        run3.stdout
    );
}

#[test]
fn dry_run_over_an_rfc3339_config_leaves_the_file_byte_identical() {
    // Round-trip stability: a dry run over the canonical form never rewrites
    // the config, so hand-written RFC 3339 survives byte-for-byte.
    let dir = TempDir::new();
    let text = floored_config(
        Some(EPOCH),
        Some(EPOCH + 1),
        "test-client-id",
        "test-client-secret",
    );
    write_file(&dir.path().join("config.toml"), &text);
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![weight_group(82400, -3, EPOCH + 60)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &[], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(config_text(&dir), text);
}

// ---------------------------------------------------------------------------
// BP floor; read-back removed (ticket 02)
// ---------------------------------------------------------------------------

#[test]
fn bp_apply_advances_bp_floor() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![bp_group(120, 80, Some(72), EPOCH)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        1
    );
    assert_eq!(floor(&dir, "bp"), Some(EPOCH));
    assert_eq!(floor(&dir, "weight"), None);
    assert!(
        run.stdout
            .contains("blood-pressure: 1 written, 0 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn bp_apply_rerun_writes_nothing() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(None, Some(EPOCH), "test-client-id", "test-client-secret"),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![bp_group(120, 80, Some(72), EPOCH)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1);
    assert_eq!(startdate(&reads[0]), EPOCH + 1);
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());
    assert_eq!(floor(&dir, "bp"), Some(EPOCH));
}

#[test]
fn two_same_day_bp_readings_both_upload() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    // Two readings on the same calendar day (2026-01-02, hours apart).
    let withings = single_page_withings(vec![
        bp_group(120, 80, Some(72), EPOCH),
        bp_group(125, 85, Some(70), EPOCH + 4 * 3600),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        2,
        "{:#?}",
        garmin.requests()
    );
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 4 * 3600));
    assert!(
        run.stdout
            .contains("blood-pressure: 2 written, 0 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn no_run_ever_calls_the_garmin_bp_range_endpoint() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = ok_garmin();

    let all = run_sync(&dir, &["all", "--apply"], &withings, &garmin);
    let bp = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);

    assert_eq!(all.code, 0, "stderr: {}", all.stderr);
    assert_eq!(bp.code, 0, "stderr: {}", bp.stderr);
    // The deleted read-back means Garmin sees writes only, never a GET.
    let gets = garmin
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .count();
    assert_eq!(gets, 0, "{:#?}", garmin.requests());
}

#[test]
fn sync_all_advances_floors_independently_on_partial_failure() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH),
            Some(EPOCH + 1),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![
        weight_group(82500, -3, EPOCH + 120),
        bp_group(120, 80, Some(72), EPOCH + 90),
    ]);
    let garmin = FakeServer::start(vec![
        Route::post("/weight-service/user-weight", |_req, _i| {
            FakeResponse::new(500, "boom")
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, _i| {
            FakeResponse::json(200, "{}")
        }),
    ]);

    let run = run_sync(&dir, &["all", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 1, "stdout: {}", run.stdout);
    // BP went through; its floor advances. The failed weight floor stays.
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        1
    );
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 90));
    assert_eq!(floor(&dir, "weight"), Some(EPOCH));
}

// ---------------------------------------------------------------------------
// Flags, backfill, and boundary semantics (ticket 03)
// ---------------------------------------------------------------------------

#[test]
fn since_flag_bypasses_floors_and_apply_advances_them() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            Some(EPOCH + 100),
            Some(EPOCH + 100),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    // Older data than the floors: a flag-driven backfill must still read it.
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(
        &dir,
        &["all", "--apply", "--since", "2026-01-01"],
        &withings,
        &garmin,
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1);
    // The flag bounds the window by itself: not floor + 1.
    assert_eq!(startdate(&reads[0]), JAN1);
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        1
    );
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        1
    );
    // A successful flag-driven apply still advances the floors.
    assert_eq!(floor(&dir, "weight"), Some(EPOCH));
    assert_eq!(floor(&dir, "bp"), Some(EPOCH + 1));
}

#[test]
fn hand_lowered_floor_resends_older_bp_reading() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            None,
            Some(EPOCH + 100),
            "test-client-id",
            "test-client-secret",
        ),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    let withings = single_page_withings(vec![bp_group(120, 80, Some(72), EPOCH)]);
    let garmin = ok_garmin();

    // Floor ahead of the data: nothing to write.
    let first = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());

    // Hand-lower the floor below the measurement: the next apply re-sends it
    // (for BP this duplicates the entry on Garmin — the documented lever).
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(
            None,
            Some(EPOCH - 10),
            "test-client-id",
            "test-client-secret",
        ),
    );
    let second = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        1,
        "{:#?}",
        garmin.requests()
    );
    assert_eq!(floor(&dir, "bp"), Some(EPOCH));
}

#[test]
fn measurement_at_exactly_the_floor_is_not_rewritten() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        &floored_config(None, Some(EPOCH), "test-client-id", "test-client-secret"),
    );
    write_file(&dir.path().join("tokens.json"), VALID_TOKENS);
    // The measurement sits exactly at the stored floor.
    let withings = single_page_withings(vec![bp_group(120, 80, Some(72), EPOCH)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["bp", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(
        startdate(&withings.requests_for("POST", "/measure")[0]),
        EPOCH + 1
    );
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());
    assert_eq!(floor(&dir, "bp"), Some(EPOCH));
}

#[test]
fn invalid_flag_window_exits_2() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![]);
    let garmin = ok_garmin();

    let run = run_sync(
        &dir,
        &["--since", "2026-02-01", "--until", "2026-01-01"],
        &withings,
        &garmin,
    );

    assert_eq!(run.code, 2, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("invalid window"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn config_write_failure_warns_but_run_outcome_unchanged() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![weight_group(82400, -3, EPOCH)]);
    let garmin = ok_garmin();
    // Make the config un-writable after load: the floor update at the end of
    // the apply must warn (on stderr) without changing the reported outcome.
    let config_path = dir.path().join("config.toml");
    std::fs::set_permissions(
        &config_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o400),
    )
    .unwrap();

    let run = run_sync(&dir, &["weight", "--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        1
    );
    assert!(
        run.stdout.contains("summary: weight synced"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("warning") && run.stderr.contains("floor"),
        "stderr: {}",
        run.stderr
    );
}
