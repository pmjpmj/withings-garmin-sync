//! Black-box tests for ADR-0006: per-service auth subcommands — `auth
//! withings` and `auth garmin` repair one service's tokens without touching
//! the other service's section of `tokens.json`.

mod common;

use common::{run_bin, run_bin_stdin, write_file, FakeResponse, FakeServer, Route, TempDir};

const CONFIG: &str =
    "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n";

/// A tokens file with both sections, standing in for an already-authenticated
/// install. The tests assert a per-service auth run replaces only its own
/// section and leaves the other one's values untouched.
const BOTH_TOKENS: &str = r#"{
  "withings": { "access_token": "old-wa", "refresh_token": "old-wr", "expires_at": 4102444800 },
  "garmin": { "access_token": "old-ga", "refresh_token": "old-gr", "client_id": "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2" }
}"#;

/// A Withings token endpoint that hands out fresh tokens on every exchange.
fn withings_token_server() -> FakeServer {
    FakeServer::start(vec![Route::post("/v2/oauth2", |_req, _i| {
        FakeResponse::json(
            200,
            r#"{"status":0,"body":{"userid":4242,"access_token":"new-wa","refresh_token":"new-wr","expires_in":10800,"scope":"user.metrics","token_type":"Bearer"}}"#,
        )
    })])
}

/// Minimal always-succeeding Garmin SSO + diauth pair.
fn garmin_servers() -> (FakeServer, FakeServer) {
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
    (sso, diauth)
}

/// Base-URL overrides that make any contact with the *other* service fail
/// fast (connection refused), so a per-service run that touches the wrong
/// service cannot slip through silently.
fn unreachable_other_service() -> Vec<(&'static str, &'static str)> {
    vec![
        ("WGS_WITHINGS_API_BASE", "http://127.0.0.1:1"),
        ("WGS_GARMIN_SSO_BASE", "http://127.0.0.1:1"),
        ("WGS_GARMIN_DIAUTH_BASE", "http://127.0.0.1:1"),
        ("WGS_GARMIN_API_BASE", "http://127.0.0.1:1"),
    ]
}

fn read_tokens(dir: &TempDir) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.path().join("tokens.json")).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// CLI shape
// ---------------------------------------------------------------------------

#[test]
fn auth_help_lists_service_subcommands() {
    let run = run_bin(&["auth", "--help"], &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    // Each subcommand gets its own entry in the Commands section (a trimmed
    // line starting with the subcommand name); matching the binary name
    // embedded elsewhere in the help is not enough.
    let commands: Vec<&str> = run.stdout.lines().map(str::trim_start).collect();
    for sub in ["withings", "garmin"] {
        assert!(
            commands.iter().any(|line| line.starts_with(sub)),
            "missing {sub} in help: {}",
            run.stdout
        );
    }
}

#[test]
fn auth_service_help_lists_flags() {
    for sub in ["withings", "garmin"] {
        let run = run_bin(&["auth", sub, "--help"], &[]);
        assert_eq!(run.code, 0, "stderr: {}", run.stderr);
        for flag in ["--config-dir", "--verbose"] {
            assert!(
                run.stdout.contains(flag),
                "missing {flag} in `auth {sub} --help`: {}",
                run.stdout
            );
        }
    }
}

#[test]
fn auth_unknown_service_exits_2() {
    let run = run_bin(&["auth", "frobnicate"], &[]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unrecognized subcommand"));
}

#[test]
fn auth_flags_conflict_with_service_subcommands() {
    // Flags belong on the subcommand (`auth withings --config-dir X`); a flag
    // before the subcommand is rejected loudly rather than silently ignored.
    let run = run_bin(&["auth", "--config-dir", "/tmp/x", "withings"], &[]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("cannot be used with"),
        "stderr: {}",
        run.stderr
    );
}

// ---------------------------------------------------------------------------
// Bare `auth` == both services
// ---------------------------------------------------------------------------

#[test]
fn bare_auth_runs_both_services_and_persists_both_sections() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    let withings = withings_token_server();
    let (sso, diauth) = garmin_servers();

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", sso.base_url.as_str()),
            ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ],
        Some("http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("auth: Withings connected"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("auth: Garmin connected"),
        "stdout: {}",
        run.stdout
    );
    let tokens = read_tokens(&dir);
    assert_eq!(tokens["withings"]["access_token"], "new-wa");
    assert_eq!(tokens["withings"]["refresh_token"], "new-wr");
    assert_eq!(tokens["garmin"]["access_token"], "new-ga");
    assert_eq!(tokens["garmin"]["refresh_token"], "new-gr");
    assert_eq!(
        tokens["garmin"]["client_id"],
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
    );
}

// ---------------------------------------------------------------------------
// Per-service runs replace only their own section
// ---------------------------------------------------------------------------

#[test]
fn auth_withings_replaces_only_the_withings_section() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_file(&dir.path().join("tokens.json"), BOTH_TOKENS);
    let withings = withings_token_server();

    // Garmin base URLs point at a dead port: a withings-only run must never
    // touch the Garmin flow.
    let mut env = unreachable_other_service();
    env.push(("WGS_WITHINGS_API_BASE", withings.base_url.as_str()));
    let run = run_bin_stdin(
        &[
            "auth",
            "withings",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ],
        &env,
        Some("http://localhost:8765/?code=abc123\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("auth: Withings connected"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("auth: Garmin connected"),
        "stdout: {}",
        run.stdout
    );

    let tokens = read_tokens(&dir);
    assert_eq!(tokens["withings"]["access_token"], "new-wa");
    assert_eq!(tokens["withings"]["refresh_token"], "new-wr");
    // The garmin section is untouched.
    assert_eq!(tokens["garmin"]["access_token"], "old-ga");
    assert_eq!(tokens["garmin"]["refresh_token"], "old-gr");
    assert_eq!(
        tokens["garmin"]["client_id"],
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
    );
}

#[test]
fn auth_garmin_needs_no_config_and_replaces_only_the_garmin_section() {
    let dir = TempDir::new();
    // No config.toml on purpose: `auth garmin` must not require one.
    write_file(&dir.path().join("tokens.json"), BOTH_TOKENS);
    let (sso, diauth) = garmin_servers();

    // The Withings base URL points at a dead port: a garmin-only run must
    // never touch the Withings flow.
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
    assert!(
        run.stdout.contains("auth: Garmin connected"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("auth: Withings connected"),
        "stdout: {}",
        run.stdout
    );

    let tokens = read_tokens(&dir);
    assert_eq!(tokens["garmin"]["access_token"], "new-ga");
    assert_eq!(tokens["garmin"]["refresh_token"], "new-gr");
    // The withings section is untouched.
    assert_eq!(tokens["withings"]["access_token"], "old-wa");
    assert_eq!(tokens["withings"]["refresh_token"], "old-wr");
    assert_eq!(tokens["withings"]["expires_at"], 4102444800u64);

    // And no config.toml appeared.
    assert!(
        !dir.path().join("config.toml").exists(),
        "auth garmin must not create or touch config.toml"
    );
}

#[test]
fn failed_auth_withings_leaves_tokens_file_byte_identical() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), CONFIG);
    write_file(&dir.path().join("tokens.json"), BOTH_TOKENS);
    let before = std::fs::read(dir.path().join("tokens.json")).unwrap();

    // The Withings token endpoint rejects the authorization code; the
    // exchange fails before any token write happens.
    let withings = FakeServer::start(vec![Route::post("/v2/oauth2", |_req, _i| {
        FakeResponse::json(200, r#"{"status": 2556}"#)
    })]);

    let mut env = unreachable_other_service();
    env.push(("WGS_WITHINGS_API_BASE", withings.base_url.as_str()));
    let run = run_bin_stdin(
        &[
            "auth",
            "withings",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ],
        &env,
        Some("http://localhost:8765/?code=bad-code\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("2556") || run.stderr.to_lowercase().contains("reject"),
        "stderr: {}",
        run.stderr
    );
    let after = std::fs::read(dir.path().join("tokens.json")).unwrap();
    assert_eq!(
        before, after,
        "a failed auth withings must leave tokens.json byte-identical"
    );
}

#[test]
fn failed_auth_garmin_leaves_tokens_file_byte_identical() {
    let dir = TempDir::new();
    write_file(&dir.path().join("tokens.json"), BOTH_TOKENS);
    let before = std::fs::read(dir.path().join("tokens.json")).unwrap();

    let sso = FakeServer::start(vec![
        Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
            FakeResponse::new(200, "ok")
        }),
        Route::post("/mobile/api/login", |_req, _i| {
            FakeResponse::json(
                200,
                r#"{"responseStatus": {"type": "INVALID_USERNAME_PASSWORD"}}"#,
            )
        }),
    ]);
    let diauth = FakeServer::start(vec![]);

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
        Some("user@example.com\nwrongpass\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr
            .to_lowercase()
            .contains("invalid username or password"),
        "stderr: {}",
        run.stderr
    );
    let after = std::fs::read(dir.path().join("tokens.json")).unwrap();
    assert_eq!(
        before, after,
        "a failed auth garmin must leave tokens.json byte-identical"
    );
}
