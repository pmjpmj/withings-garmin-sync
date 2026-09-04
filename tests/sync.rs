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

/// Garmin API fake that accepts weight (204) and BP (200) writes, and answers
/// the BP range read-back (ticket 12) with "no existing measurements".
fn ok_garmin() -> FakeServer {
    FakeServer::start(vec![
        Route::post("/weight-service/user-weight", |_req, _i| {
            FakeResponse::new(204, "")
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, _i| {
            FakeResponse::json(200, "{}")
        }),
        Route::get_prefix("/bloodpressure-service/bloodpressure/range", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"from":"2026-01-02","until":"2026-01-02","measurementSummaries":[]}"#,
            )
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
fn apply_weight_write_sends_exact_header_set_without_extras() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![weight_group(82400, -3, EPOCH)]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let weight_calls = garmin.requests_for("POST", "/weight-service/user-weight");
    assert_eq!(weight_calls.len(), 1, "{:#?}", garmin.requests());

    // The exact header set the write path sends (plus the HTTP-level headers
    // reqwest/hyper add: host and content-length). Anything else here is an
    // extra Garmin never saw in the spike's proven curl.
    let mut names: Vec<String> = weight_calls[0]
        .headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect();
    names.sort();
    // Deliberately no dedup: a duplicate header line (e.g. Content-Type sent
    // twice) must fail this test — Garmin's edge rejects it with a 400.
    assert_eq!(names.len(), weight_calls[0].headers.len());
    let expected = [
        "accept",
        "authorization",
        "content-length",
        "content-type",
        "host",
        "user-agent",
        "x-app-ver",
        "x-garmin-client-platform",
        "x-garmin-paired-app-version",
        "x-garmin-user-agent",
        "x-gcexperience",
        "x-lang",
    ];
    assert_eq!(
        names,
        expected.to_vec(),
        "unexpected header set: {:#?}",
        weight_calls[0].headers
    );

    // The required values, exactly as the spike's working curl sent them.
    assert_eq!(
        weight_calls[0].header("user-agent").unwrap(),
        "GCM-Android-5.23"
    );
    assert_eq!(weight_calls[0].header("x-app-ver").unwrap(), "10861");
    assert_eq!(
        weight_calls[0]
            .header("x-garmin-paired-app-version")
            .unwrap(),
        "10861"
    );
    assert_eq!(
        weight_calls[0].header("x-garmin-client-platform").unwrap(),
        "Android"
    );
    assert_eq!(weight_calls[0].header("x-lang").unwrap(), "en");
    assert_eq!(weight_calls[0].header("x-gcexperience").unwrap(), "GC5");
    assert_eq!(
        weight_calls[0].header("accept").unwrap(),
        "application/json"
    );
    assert!(
        weight_calls[0]
            .header("content-type")
            .unwrap()
            .starts_with("application/json"),
        "content-type: {:?}",
        weight_calls[0].header("content-type")
    );
}

#[test]
fn apply_writes_send_exact_headers_and_bodies_for_both_endpoints() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    let garmin = ok_garmin();

    let run = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // The full wire request each write must produce: the native header set
    // with exact values, no extras, and the exact JSON body. This is the
    // shape the spike's curl proved live (ticket 01) and the ticket-11
    // investigation found Garmin's edge to be strict about.
    let expected_header_names = [
        "accept",
        "authorization",
        "content-length",
        "content-type",
        "host",
        "user-agent",
        "x-app-ver",
        "x-garmin-client-platform",
        "x-garmin-paired-app-version",
        "x-garmin-user-agent",
        "x-gcexperience",
        "x-lang",
    ];
    let expected_values: [(&str, &str); 9] = [
        ("authorization", "Bearer ga"),
        ("user-agent", "GCM-Android-5.23"),
        (
            "x-garmin-user-agent",
            "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0",
        ),
        ("x-garmin-paired-app-version", "10861"),
        ("x-garmin-client-platform", "Android"),
        ("x-app-ver", "10861"),
        ("x-lang", "en"),
        ("x-gcexperience", "GC5"),
        ("accept", "application/json"),
    ];

    let cases: [(&str, &str, serde_json::Value); 2] = [
        (
            "/weight-service/user-weight",
            "weight",
            json!({
                "dateTimestamp": "2026-01-02T08:30:00.000",
                "gmtTimestamp": "2026-01-02T08:30:00.000",
                "unitKey": "kg",
                "sourceType": "MANUAL",
                "value": 82.4
            }),
        ),
        (
            "/bloodpressure-service/bloodpressure",
            "blood-pressure",
            json!({
                "measurementTimestampLocal": "2026-01-02T08:30:01.000",
                "measurementTimestampGMT": "2026-01-02T08:30:01.000",
                "systolic": 120,
                "diastolic": 80,
                "pulse": 72,
                "sourceType": "MANUAL"
            }),
        ),
    ];

    for (path, label, expected_body) in cases {
        let calls = garmin.requests_for("POST", path);
        assert_eq!(calls.len(), 1, "{label}: {:#?}", garmin.requests());
        let request = &calls[0];

        let mut names: Vec<String> = request
            .headers
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        names.sort();
        // Deliberately no dedup: duplicate header lines must fail the test.
        assert_eq!(names.len(), request.headers.len(), "{label}: duplicates");
        assert_eq!(
            names,
            expected_header_names.to_vec(),
            "{label}: unexpected header set: {:#?}",
            request.headers
        );

        for (name, value) in expected_values {
            assert_eq!(request.header(name), Some(value), "{label}: header {name}");
        }
        assert!(
            request
                .header("content-type")
                .unwrap()
                .starts_with("application/json"),
            "{label}: content-type: {:?}",
            request.header("content-type")
        );
        assert_eq!(
            request.header("host").unwrap(),
            garmin.base_url.trim_start_matches("http://"),
            "{label}: host"
        );
        assert_eq!(
            request.header("content-length").unwrap(),
            request.body.len().to_string(),
            "{label}: content-length must match the body bytes"
        );

        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body, expected_body, "{label}: request body");
    }
}

/// Capture the *actual bytes* the binary puts on a real TCP socket for the
/// weight write — nothing parsed, nothing reconstructed — and check the wire
/// request against the proven spike curl byte-for-byte. This guards against
/// transport-level drift (extra headers, casing, framing) that parsed-view
/// tests could hide.
#[test]
fn apply_weight_write_raw_wire_bytes_match_proven_request() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let raw_addr = listener.local_addr().unwrap();

    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![weight_group(82400, -3, EPOCH)]);

    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let (mut stream, _peer) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the binary never connected to the wire-capture socket"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("accept failed: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();

        let mut bytes: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        let header_end = loop {
            if let Some(pos) = find_bytes(&bytes, b"\r\n\r\n") {
                break pos;
            }
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "connection closed before the headers arrived");
            bytes.extend_from_slice(&buf[..n]);
        };
        let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let content_length: usize = head
            .split("\r\n")
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
            })
            .expect("no content-length header");
        while bytes.len() < header_end + 4 + content_length {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "connection closed before the body arrived");
            bytes.extend_from_slice(&buf[..n]);
        }
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\n\r\n")
            .unwrap();
        bytes
    });

    let raw_base = format!("http://{raw_addr}");
    let env: Vec<(&str, &str)> = vec![
        ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
        ("WGS_GARMIN_API_BASE", raw_base.as_str()),
        ("TZ", "UTC"),
    ];
    let full: Vec<&str> = vec![
        "sync",
        "--config-dir",
        dir.path().to_str().unwrap(),
        "--apply",
    ];
    let run = run_bin(&full, &env);
    let bytes = server.join().unwrap();

    assert_eq!(
        run.code, 0,
        "stdout: {}\nstderr: {}",
        run.stdout, run.stderr
    );
    let text = String::from_utf8(bytes).unwrap();

    // Request line, verbatim.
    assert_eq!(
        text.split("\r\n").next().unwrap(),
        "POST /weight-service/user-weight HTTP/1.1",
        "wire request line: {text:?}"
    );

    // Every header line on the wire, verbatim: the exact set, nothing extra.
    let header_lines: Vec<&str> = text
        .split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .collect();
    let mut names: Vec<String> = header_lines
        .iter()
        .map(|line| line.split(':').next().unwrap().trim().to_ascii_lowercase())
        .collect();
    names.sort();
    let expected_names = [
        "accept",
        "authorization",
        "content-length",
        "content-type",
        "host",
        "user-agent",
        "x-app-ver",
        "x-garmin-client-platform",
        "x-garmin-paired-app-version",
        "x-garmin-user-agent",
        "x-gcexperience",
        "x-lang",
    ];
    assert_eq!(
        names,
        expected_names.to_vec(),
        "wire header lines: {header_lines:?}"
    );

    let expected_values: [(&str, &str); 10] = [
        ("authorization", "Bearer ga"),
        ("user-agent", "GCM-Android-5.23"),
        (
            "x-garmin-user-agent",
            "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0",
        ),
        ("x-garmin-paired-app-version", "10861"),
        ("x-garmin-client-platform", "Android"),
        ("x-app-ver", "10861"),
        ("x-lang", "en"),
        ("x-gcexperience", "GC5"),
        ("accept", "application/json"),
        ("content-type", "application/json"),
    ];
    for (name, value) in expected_values {
        let line = header_lines
            .iter()
            .find(|line| {
                line.split_once(':')
                    .unwrap()
                    .0
                    .trim()
                    .eq_ignore_ascii_case(name)
            })
            .unwrap_or_else(|| panic!("missing wire header {name}: {header_lines:?}"));
        assert_eq!(
            line.split_once(':').unwrap().1.trim(),
            value,
            "wire header {name}"
        );
    }
    assert_eq!(
        header_lines
            .iter()
            .find(|l| l
                .split_once(':')
                .unwrap()
                .0
                .trim()
                .eq_ignore_ascii_case("host"))
            .unwrap()
            .split_once(':')
            .unwrap()
            .1
            .trim(),
        raw_addr.to_string(),
        "wire host header"
    );

    // The body bytes, verbatim.
    let expected_body = json!({
        "dateTimestamp": "2026-01-02T08:30:00.000",
        "gmtTimestamp": "2026-01-02T08:30:00.000",
        "unitKey": "kg",
        "sourceType": "MANUAL",
        "value": 82.4
    })
    .to_string();
    let body = text.split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(body, expected_body, "wire body bytes");
    assert_eq!(
        header_lines
            .iter()
            .find(|l| l
                .split_once(':')
                .unwrap()
                .0
                .trim()
                .eq_ignore_ascii_case("content-length"))
            .unwrap()
            .split_once(':')
            .unwrap()
            .1
            .trim(),
        expected_body.len().to_string(),
        "wire content-length"
    );
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
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
        Route::get_prefix("/bloodpressure-service/bloodpressure/range", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"from":"2026-01-02","until":"2026-01-02","measurementSummaries":[]}"#,
            )
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
fn apply_rerun_skips_bp_days_already_on_garmin_but_rewrites_weight() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = single_page_withings(vec![
        weight_group(82400, -3, EPOCH),
        bp_group(120, 80, Some(72), EPOCH + 1),
    ]);
    // The BP read-back is stateful: the first read sees no existing
    // measurements, the second sees the day already populated (as a live
    // Garmin account would after the first write).
    let garmin = FakeServer::start(vec![
        Route::post("/weight-service/user-weight", |_req, _i| {
            FakeResponse::new(204, "")
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, _i| {
            FakeResponse::json(200, "{}")
        }),
        Route::get_prefix(
            "/bloodpressure-service/bloodpressure/range",
            |_req, call| {
                if call == 0 {
                    FakeResponse::json(
                        200,
                        r#"{"from":"2026-01-02","until":"2026-01-02","measurementSummaries":[]}"#,
                    )
                } else {
                    FakeResponse::json(
                        200,
                        r#"{"from":"2026-01-02","until":"2026-01-02","measurementSummaries":[{"startDate":"2026-01-02","endDate":"2026-01-02","highSystolic":120,"highDiastolic":80,"lowSystolic":120,"lowDiastolic":80,"numOfMeasurements":1,"category":"STAGE_1_HIGH","categoryName":"NORMAL","measurements":[]}]}"#,
                    )
                }
            },
        ),
    ]);

    let first = run_sync(&dir, &["--apply"], &withings, &garmin);
    let second = run_sync(&dir, &["--apply"], &withings, &garmin);

    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);

    // Weight still writes every run (its endpoint dedups by timestamp); the
    // BP write happens once — the second run's read-back sees the day and
    // skips the re-write.
    let weight_calls = garmin.requests_for("POST", "/weight-service/user-weight");
    let bp_calls = garmin.requests_for("POST", "/bloodpressure-service/bloodpressure");
    assert_eq!(weight_calls.len(), 2, "{:#?}", garmin.requests());
    assert_eq!(bp_calls.len(), 1, "{:#?}", garmin.requests());
    assert_eq!(weight_calls[0].body, weight_calls[1].body);
    // The second run reports the BP reading as skipped, not written.
    assert!(
        second
            .stdout
            .contains("blood-pressure: 0 written, 1 skipped, 0 failed"),
        "stdout: {}",
        second.stdout
    );
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
