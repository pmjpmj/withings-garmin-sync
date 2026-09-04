//! Black-box tests for ticket 08: resilience (token refresh, retry with
//! backoff, the EU consent gate) and observability (`--verbose` request logs).

mod common;

use common::{
    form_value, parse_form, run_bin, write_file, FakeResponse, FakeServer, Route, TempDir,
};
use serde_json::json;

const CONFIG: &str =
    "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n";

/// tokens.json with the given fields for the two providers.
fn write_tokens(dir: &TempDir, withings_access: &str, withings_expires: u64, garmin_access: &str) {
    write_file(
        &dir.path().join("tokens.json"),
        &json!({
            "withings": {
                "access_token": withings_access,
                "refresh_token": "wa-refresh",
                "expires_at": withings_expires
            },
            "garmin": {
                "access_token": garmin_access,
                "refresh_token": "ga-refresh",
                "client_id": "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
            }
        })
        .to_string(),
    );
}

fn read_tokens(dir: &TempDir) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.path().join("tokens.json")).unwrap()).unwrap()
}

fn empty_measures() -> FakeResponse {
    FakeResponse::json(
        200,
        r#"{"status":0,"body":{"updatetime":1767342600,"timezone":"UTC","measuregrps":[],"more":0,"offset":0}}"#,
    )
}

fn measure_page() -> FakeResponse {
    FakeResponse::json(
        200,
        r#"{"status":0,"body":{"updatetime":1767342600,"timezone":"UTC","measuregrps":[{"grpid":1,"attrib":2,"date":1767342600,"category":1,"measures":[{"value":82400,"type":1,"unit":-3}]}],"more":0,"offset":0}}"#,
    )
}

fn sync_env<'a>(
    withings: &'a FakeServer,
    garmin_api: &'a FakeServer,
    diauth: &'a FakeServer,
) -> Vec<(&'static str, &'a str)> {
    vec![
        ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
        ("WGS_GARMIN_API_BASE", garmin_api.base_url.as_str()),
        ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ("TZ", "UTC"),
    ]
}

fn run_sync<'a>(dir: &'a TempDir, args: &[&str], env: &[(&str, &'a str)]) -> common::Run {
    let mut full: Vec<&str> = vec!["sync", "--config-dir", dir.path().to_str().unwrap()];
    full.extend_from_slice(args);
    run_bin(&full, env)
}

// ---------------------------------------------------------------------------
// Withings token refresh
// ---------------------------------------------------------------------------

#[test]
fn expired_withings_token_is_refreshed_before_reads() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa-stale", 1, "ga"); // expires_at = 1: long past
    let withings = FakeServer::start(vec![
        Route::post("/v2/oauth2", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"status":0,"body":{"access_token":"wa-new","refresh_token":"wa-refresh-2","expires_in":10800,"scope":"user.metrics","token_type":"Bearer"}}"#,
            )
        }),
        Route::post("/measure", |_req, _i| empty_measures()),
    ]);
    let garmin = FakeServer::start(vec![]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &[], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // Refresh happens first, with the exact refresh form.
    let requests = withings.requests();
    assert_eq!(requests.len(), 2, "{requests:#?}");
    assert_eq!(requests[0].path, "/v2/oauth2");
    let fields = parse_form(&requests[0].body);
    assert_eq!(form_value(&fields, "grant_type"), "refresh_token");
    assert_eq!(form_value(&fields, "refresh_token"), "wa-refresh");
    assert_eq!(form_value(&fields, "client_id"), "test-client-id");
    assert_eq!(form_value(&fields, "client_secret"), "test-client-secret");

    // The read uses the fresh token.
    assert_eq!(requests[1].path, "/measure");
    assert_eq!(
        requests[1].header("authorization").unwrap(),
        "Bearer wa-new",
        "headers: {:?}",
        requests[1].headers
    );

    // The refreshed tokens (and new expiry) are persisted.
    let tokens = read_tokens(&dir);
    assert_eq!(tokens["withings"]["access_token"], "wa-new");
    assert_eq!(tokens["withings"]["refresh_token"], "wa-refresh-2");
    let expires = tokens["withings"]["expires_at"].as_u64().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(expires > now + 10000, "expiry {expires} not in the future");
}

#[test]
fn rejected_withings_refresh_exits_4_and_tells_to_reauth() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa-stale", 1, "ga");
    let withings = FakeServer::start(vec![Route::post("/v2/oauth2", |_req, _i| {
        FakeResponse::json(200, r#"{"status": 2557}"#)
    })]);
    let garmin = FakeServer::start(vec![]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &[], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("re-run `auth`"),
        "stderr: {}",
        run.stderr
    );
}

// ---------------------------------------------------------------------------
// Garmin 401 -> refresh -> single retry
// ---------------------------------------------------------------------------

#[test]
fn garmin_401_write_refreshes_token_and_retries_once() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga-stale");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| measure_page())]);
    let garmin = FakeServer::start(vec![Route::post(
        "/weight-service/user-weight",
        |req, call| {
            if call == 0 {
                assert_eq!(req.header("authorization").unwrap(), "Bearer ga-stale");
                FakeResponse::new(401, "unauthorized")
            } else {
                assert_eq!(req.header("authorization").unwrap(), "Bearer ga-new");
                FakeResponse::new(204, "")
            }
        },
    )]);
    let diauth = FakeServer::start(vec![Route::post(
        "/di-oauth2-service/oauth/token",
        |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"access_token":"ga-new","refresh_token":"ga-refresh-2"}"#,
            )
        },
    )]);

    let run = run_sync(&dir, &["--apply"], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // Exactly one refresh, with the refresh grant.
    let refreshes = diauth.requests_for("POST", "/di-oauth2-service/oauth/token");
    assert_eq!(refreshes.len(), 1);
    let fields = parse_form(&refreshes[0].body);
    assert_eq!(form_value(&fields, "grant_type"), "refresh_token");
    assert_eq!(form_value(&fields, "refresh_token"), "ga-refresh");

    // The write went out twice: stale token (401), fresh token (204).
    let writes = garmin.requests_for("POST", "/weight-service/user-weight");
    assert_eq!(writes.len(), 2, "{writes:#?}");

    // The rotated tokens are persisted.
    let tokens = read_tokens(&dir);
    assert_eq!(tokens["garmin"]["access_token"], "ga-new");
    assert_eq!(tokens["garmin"]["refresh_token"], "ga-refresh-2");
}

#[test]
fn rejected_garmin_refresh_aborts_with_exit_4() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga-stale");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| measure_page())]);
    let garmin = FakeServer::start(vec![Route::post(
        "/weight-service/user-weight",
        |_req, _i| FakeResponse::new(401, "unauthorized"),
    )]);
    let diauth = FakeServer::start(vec![Route::post(
        "/di-oauth2-service/oauth/token",
        |_req, _i| FakeResponse::new(401, r#"{"error":"invalid_grant"}"#),
    )]);

    let run = run_sync(&dir, &["--apply"], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("re-run `auth`"),
        "stderr: {}",
        run.stderr
    );
    // No second write attempt after the failed refresh.
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Retry with backoff: Withings 601, Garmin 429 (HTTP and JSON)
// ---------------------------------------------------------------------------

#[test]
fn withings_601_is_retried_with_backoff() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, call| {
        if call == 0 {
            FakeResponse::new(601, "rate limited")
        } else {
            empty_measures()
        }
    })]);
    let garmin = FakeServer::start(vec![]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &[], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(withings.requests_for("POST", "/measure").len(), 2);
}

#[test]
fn garmin_429_is_retried_whether_http_status_or_json_body() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| {
        FakeResponse::json(
            200,
            r#"{"status":0,"body":{"updatetime":1767342600,"timezone":"UTC","measuregrps":[
                {"grpid":1,"attrib":2,"date":1767342600,"category":1,"measures":[{"value":82400,"type":1,"unit":-3}]},
                {"grpid":2,"attrib":2,"date":1767342601,"category":1,"measures":[{"value":120,"type":10,"unit":0},{"value":80,"type":9,"unit":0}]}
            ],"more":0,"offset":0}}"#,
        )
    })]);
    // Weight: HTTP 429 then success. BP: 200 body with JSON 429 then success.
    let garmin = FakeServer::start(vec![
        Route::post("/weight-service/user-weight", |_req, call| {
            if call == 0 {
                FakeResponse::new(429, "slow down")
            } else {
                FakeResponse::new(204, "")
            }
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, call| {
            if call == 0 {
                FakeResponse::json(200, r#"{"error":{"status-code":"429"}}"#)
            } else {
                FakeResponse::json(200, "{}")
            }
        }),
        Route::get_prefix("/bloodpressure-service/bloodpressure/range", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"from":"2026-01-02","until":"2026-01-02","measurementSummaries":[]}"#,
            )
        }),
    ]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &["--apply"], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        2
    );
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        2
    );
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
}

// ---------------------------------------------------------------------------
// The EU consent gate
// ---------------------------------------------------------------------------

#[test]
fn garmin_412_surfaces_the_upload_consent_message() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| measure_page())]);
    let garmin = FakeServer::start(vec![Route::post(
        "/weight-service/user-weight",
        |_req, _i| FakeResponse::new(412, r#"{"error":"consent required"}"#),
    )]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &["--apply"], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 1, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("upload consent"),
        "stderr: {}",
        run.stderr
    );
    // 412 is not retried.
    assert_eq!(
        garmin
            .requests_for("POST", "/weight-service/user-weight")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Verbose request logging
// ---------------------------------------------------------------------------

#[test]
fn verbose_logs_each_request_and_response() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| empty_measures())]);
    let garmin = FakeServer::start(vec![]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &["--verbose"], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("[verbose] -> POST"),
        "stderr: {}",
        run.stderr
    );
    assert!(run.stderr.contains("/measure"), "stderr: {}", run.stderr);
    assert!(run.stderr.contains("<- HTTP 200"), "stderr: {}", run.stderr);
}

#[test]
fn persistent_json_429_is_failure_not_success() {
    // A 200 whose JSON body keeps saying 429 must never be counted as a
    // successful blood-pressure write.
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| {
        FakeResponse::json(
            200,
            r#"{"status":0,"body":{"updatetime":1767342600,"timezone":"UTC","measuregrps":[
                {"grpid":2,"attrib":2,"date":1767342601,"category":1,"measures":[{"value":120,"type":10,"unit":0},{"value":80,"type":9,"unit":0}]}
            ],"more":0,"offset":0}}"#,
        )
    })]);
    let garmin = FakeServer::start(vec![
        Route::get_prefix("/bloodpressure-service/bloodpressure/range", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"from":"2026-01-02","until":"2026-01-02","measurementSummaries":[]}"#,
            )
        }),
        Route::post("/bloodpressure-service/bloodpressure", |_req, _i| {
            FakeResponse::json(200, r#"{"error":{"status-code":"429"}}"#)
        }),
    ]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &["--apply"], &sync_env(&withings, &garmin, &diauth));

    // Three attempts (backoff), then a counted failure: exit 1, not 0.
    assert_eq!(run.code, 1, "stdout: {}", run.stdout);
    assert_eq!(
        garmin
            .requests_for("POST", "/bloodpressure-service/bloodpressure")
            .len(),
        3
    );
    assert!(
        run.stdout
            .contains("blood-pressure: 0 written, 0 skipped, 1 failed"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn empty_tokens_file_exits_3_with_run_auth_hint() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_file(&dir.path().join("tokens.json"), "{}");
    let withings = FakeServer::start(vec![]);
    let garmin = FakeServer::start(vec![]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(&dir, &[], &sync_env(&withings, &garmin, &diauth));

    assert_eq!(run.code, 3, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("run `auth` first"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn until_alone_means_beginning_despite_config_since() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n\n[sync]\nsince = \"2026-01-15\"\n",
    );
    write_tokens(&dir, "wa", now_epoch_plus(3600), "ga");
    let withings = FakeServer::start(vec![Route::post("/measure", |_req, _i| empty_measures())]);
    let garmin = FakeServer::start(vec![]);
    let diauth = FakeServer::start(vec![]);

    let run = run_sync(
        &dir,
        &["--until", "2026-02-01"],
        &sync_env(&withings, &garmin, &diauth),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reads = withings.requests_for("POST", "/measure");
    let fields = parse_form(&reads[0].body);
    assert_eq!(form_value(&fields, "startdate"), "0");
    assert_eq!(form_value(&fields, "enddate"), "1769904000");
}

fn now_epoch_plus(seconds: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + seconds
}
