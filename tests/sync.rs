//! Black-box tests for ticket 07: `sync` reading body weight and blood
//! pressure from Withings and writing them to Garmin Connect (dry-run by
//! default, writes only under `--apply`), driven through the binary against
//! fake Withings and Garmin servers.

mod common;

use common::{
    form_value, parse_form, run_bin, write_valid_config_and_tokens, FakeResponse, FakeServer,
    Route, TempDir,
};
use serde_json::json;

/// 2026-01-02T08:30:00Z — a Withings measurement timestamp.
const EPOCH: i64 = 1767342600;
/// 2026-01-01T00:00:00Z.
const JAN1: i64 = 1767225600;
/// 2026-02-01T00:00:00Z.
const FEB1: i64 = 1769904000;

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

/// Garmin API fake that accepts weight (204) and BP (200) writes.
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
    let mut full: Vec<&str> = vec!["sync", "--config-dir", dir.path().to_str().unwrap()];
    full.extend_from_slice(args);
    run_bin(&full, &sync_env(withings, garmin))
}

// ---------------------------------------------------------------------------
// dry-run
// ---------------------------------------------------------------------------

#[test]
fn dry_run_reads_window_and_reports_would_writes_without_writing() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &[], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // The read request: getmeas with the metric filter, window, and Bearer.
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 1, "{:#?}", withings.requests());
    let fields = parse_form(&reads[0].body);
    assert_eq!(form_value(&fields, "action"), "getmeas");
    assert_eq!(form_value(&fields, "meastypes"), "1,9,10,11");
    let start: i64 = form_value(&fields, "startdate").parse().unwrap();
    let end: i64 = form_value(&fields, "enddate").parse().unwrap();
    let n = now();
    assert!(
        (n - 30 * 86400 - start).abs() < 120,
        "startdate {start} not ~30d ago"
    );
    assert!((n - end).abs() < 120, "enddate {end} not ~now");
    assert_eq!(
        reads[0].header("authorization").unwrap(),
        "Bearer wa",
        "headers: {:?}",
        reads[0].headers
    );

    // The would-write report: metric, value, timestamp; decoded via 10^unit.
    assert!(run.stdout.contains("dry-run"), "stdout: {}", run.stdout);
    assert!(
        run.stdout
            .contains("would write 1 weight and 1 blood-pressure measurements"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("weight: 82.4 kg at 2026-01-02T08:30:00.000"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("blood-pressure: 120/80 (pulse 72) at 2026-01-02T08:30:01.000"),
        "stdout: {}",
        run.stdout
    );

    // Dry run performs no writes whatsoever.
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());
}

#[test]
fn dry_run_follows_pagination_until_more_is_zero() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, call| {
        if call == 0 {
            FakeResponse::json(
                200,
                &measures_page(
                    serde_json::Value::Array(vec![weight_group(80000, -3, EPOCH)]),
                    1,
                    5,
                ),
            )
        } else {
            FakeResponse::json(
                200,
                &measures_page(
                    serde_json::Value::Array(vec![weight_group(81000, -3, EPOCH + 60)]),
                    0,
                    0,
                ),
            )
        }
    })]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &[], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    assert_eq!(reads.len(), 2, "{:#?}", withings.requests());
    assert_eq!(form_value(&parse_form(&reads[0].body), "offset"), "0");
    assert_eq!(form_value(&parse_form(&reads[1].body), "offset"), "5");
    assert!(
        run.stdout.contains("would write 2 weight"),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stdout.contains("80"), "stdout: {}", run.stdout);
    assert!(run.stdout.contains("81"), "stdout: {}", run.stdout);
}

#[test]
fn sync_since_until_flags_bound_the_window() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![]);
    let garmin = ok_garmin();

    let run = run_sync(
        &dir,
        &["--since", "2026-01-01", "--until", "2026-02-01"],
        &withings,
        &garmin,
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    let fields = parse_form(&reads[0].body);
    assert_eq!(form_value(&fields, "startdate"), JAN1.to_string());
    assert_eq!(form_value(&fields, "enddate"), FEB1.to_string());
    assert!(
        run.stdout.contains("2026-01-01..2026-02-01"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn sync_invalid_date_exits_2() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--since", "not-a-date"], &withings, &garmin);

    assert_eq!(run.code, 2, "stdout: {}", run.stdout);
    assert!(run.stderr.contains("not-a-date"), "stderr: {}", run.stderr);
}

// ---------------------------------------------------------------------------
// apply
// ---------------------------------------------------------------------------

#[test]
fn apply_writes_weight_and_bp_with_native_headers() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    let weight_calls = garmin.requests_for("POST", "/weight-service/user-weight");
    assert_eq!(weight_calls.len(), 1, "{:#?}", garmin.requests());
    assert_eq!(
        weight_calls[0].header("authorization").unwrap(),
        "Bearer ga"
    );
    assert_eq!(
        weight_calls[0].header("user-agent").unwrap(),
        "GCM-Android-5.23"
    );
    assert_eq!(
        weight_calls[0].header("x-garmin-client-platform").unwrap(),
        "Android"
    );
    assert!(
        weight_calls[0]
            .header("content-type")
            .unwrap()
            .contains("application/json"),
        "content-type: {:?}",
        weight_calls[0].header("content-type")
    );
    let weight_payload: serde_json::Value = serde_json::from_str(&weight_calls[0].body).unwrap();
    assert_eq!(
        weight_payload,
        json!({
            "dateTimestamp": "2026-01-02T08:30:00.000",
            "gmtTimestamp": "2026-01-02T08:30:00.000",
            "unitKey": "kg",
            "sourceType": "MANUAL",
            "value": 82.4
        })
    );

    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    assert_eq!(bp_calls.len(), 1);
    assert_eq!(bp_calls[0].header("authorization").unwrap(), "Bearer ga");
    let bp_payload: serde_json::Value = serde_json::from_str(&bp_calls[0].body).unwrap();
    assert_eq!(
        bp_payload,
        json!({
            "measurementTimestampLocal": "2026-01-02T08:30:01.000",
            "measurementTimestampGMT": "2026-01-02T08:30:01.000",
            "systolic": 120,
            "diastolic": 80,
            "pulse": 72,
            "sourceType": "MANUAL"
        })
    );

    // The report counts and one-line summary.
    assert!(
        run.stdout
            .contains("weight: 1 written, 0 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("blood-pressure: 1 written, 0 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("summary: all metrics synced"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn apply_pulse_omitted_when_withings_has_none() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![bp_group(120, 80, None, EPOCH)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    let bp_payload: serde_json::Value = serde_json::from_str(&bp_calls[0].body).unwrap();
    assert!(bp_payload.get("pulse").is_none(), "payload: {bp_payload}");
}

#[test]
fn apply_skips_out_of_range_bp_with_warning() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        bp_group(300, 80, Some(72), EPOCH), // systolic out of range -> whole record skipped
        bp_group(120, 20, Some(72), EPOCH), // diastolic out of range -> whole record skipped
        bp_group(120, 80, Some(10), EPOCH), // pulse out of range -> whole record skipped
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("out of range"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("300") && run.stderr.contains("20"),
        "stderr: {}",
        run.stderr
    );
    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    assert_eq!(bp_calls.len(), 0, "{:#?}", garmin.requests());
    assert!(
        run.stdout
            .contains("blood-pressure: 0 written, 3 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn apply_metrics_are_independent_weight_failure_does_not_block_bp() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = FakeServer::start(vec![
        Route::post("/weight-service/user-weight", |_req, _i| {
            FakeResponse::new(500, "boom")
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, _i| {
            FakeResponse::json(200, "{}")
        }),
    ]);

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    // Partial failure -> exit 1, but the BP write still went through.
    assert_eq!(run.code, 1, "stdout: {}", run.stdout);
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        1
    );
    assert!(
        run.stdout
            .contains("weight: 0 written, 0 skipped, 1 failed"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("blood-pressure: 1 written, 0 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("summary: 1 metric(s) failed"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn apply_rerun_sends_identical_writes_full_overwrite() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = ok_garmin();

    let first = run_sync(&dir, &["--apply"], &withings, &garmin);
    let second = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);

    let weight_calls = garmin.requests_for("POST", "/weight-service/user-weight");
    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    assert_eq!(weight_calls.len(), 2, "{:#?}", garmin.requests());
    assert_eq!(bp_calls.len(), 2);
    // Identical payloads both runs: Garmin's timestamp dedup makes this safe.
    assert_eq!(weight_calls[0].body, weight_calls[1].body);
    assert_eq!(bp_calls[0].body, bp_calls[1].body);
}

#[test]
fn sync_withings_read_failure_exits_1() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| {
        FakeResponse::new(500, "boom")
    })]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 1, "stdout: {}", run.stdout);
    assert!(run.stderr.contains("500"), "stderr: {}", run.stderr);
}

#[test]
fn sync_bp_without_pairing_is_skipped_with_warning() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    // A group with only systolic and no diastolic cannot form a BP record.
    let withings = single_page_withings(vec![json!({
        "grpid": 3,
        "attrib": 2,
        "date": EPOCH,
        "category": 1,
        "measures": [{"value": 120, "type": 10, "unit": 0}]
    })]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(garmin.requests().is_empty(), "{:#?}", garmin.requests());
    assert!(
        run.stderr.contains("skipping") || run.stderr.contains("incomplete"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stdout
            .contains("blood-pressure: 0 written, 1 skipped, 0 failed"),
        "stdout: {}",
        run.stdout
    );
}
