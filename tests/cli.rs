//! Black-box tests for the walking-skeleton CLI (ticket 04).
//!
//! These drive the compiled binary against a temp config dir and assert only on
//! observable behavior: stdout/stderr text, exit codes, and the files the
//! binary writes (including their permissions). No internals are reached into.

mod common;

use common::{
    run_bin, write_file, write_valid_config_and_tokens, FakeResponse, FakeServer, Route, TempDir,
    VALID_CONFIG,
};

#[test]
fn help_lists_subcommands_and_flags() {
    let run = run_bin(&["--help"], &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("auth"));
    assert!(run.stdout.contains("sync"));

    let run = run_bin(&["sync", "--help"], &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    for flag in [
        "--apply",
        "--dry-run",
        "--since",
        "--until",
        "--config-dir",
        "--verbose",
    ] {
        assert!(
            run.stdout.contains(flag),
            "missing {flag} in help: {}",
            run.stdout
        );
    }
}

#[test]
fn version_prints_name_and_version() {
    let run = run_bin(&["--version"], &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("withings-garmin-sync"));
    assert!(run.stdout.contains("0.1.0"));
}

#[test]
fn unknown_subcommand_exits_2() {
    let run = run_bin(&["frobnicate"], &[]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unrecognized subcommand"));
}

#[test]
fn unknown_flag_exits_2() {
    let run = run_bin(&["sync", "--nope"], &[]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unexpected argument"));
}

#[test]
fn apply_conflicts_with_dry_run() {
    let run = run_bin(&["sync", "--apply", "--dry-run"], &[]);
    assert_eq!(run.code, 2);
}

#[test]
fn sync_with_missing_config_exits_3_and_says_run_auth_first() {
    let dir = TempDir::new();
    let run = run_bin(&["sync", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 3);
    assert!(
        run.stderr.contains("run `auth` first"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("config not found"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn sync_with_invalid_toml_config_exits_3() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), "this is not toml {{{");
    let run = run_bin(&["sync", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 3);
    assert!(
        run.stderr.contains("run `auth` first"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn sync_with_empty_credentials_exits_3() {
    let dir = TempDir::new();
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"\"\nclient_secret = \"\"\n",
    );
    let run = run_bin(&["sync", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 3);
    assert!(run.stderr.contains("client_id"), "stderr: {}", run.stderr);
}

#[test]
fn sync_with_missing_tokens_exits_3() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), VALID_CONFIG);
    let run = run_bin(&["sync", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 3);
    assert!(
        run.stderr.contains("run `auth` first"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("tokens not found"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn sync_with_invalid_tokens_exits_3() {
    let dir = TempDir::new();
    write_file(&dir.path().join("config.toml"), VALID_CONFIG);
    write_file(&dir.path().join("tokens.json"), "not json");
    let run = run_bin(&["sync", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 3);
    assert!(
        run.stderr.contains("run `auth` first"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("invalid tokens"),
        "stderr: {}",
        run.stderr
    );
}

/// A Withings fake with an empty measurement page, so `sync` tests that
/// exercise the CLI surface (not the sync logic) never touch the network.
fn empty_withings() -> FakeServer {
    FakeServer::start(vec![Route::post("/measure", |_req, _i| {
        FakeResponse::json(
            200,
            r#"{"status":0,"body":{"updatetime":1767342600,"timezone":"UTC","measuregrps":[],"more":0,"offset":0}}"#,
        )
    })])
}

#[test]
fn sync_valid_config_dry_run_succeeds_by_default() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = empty_withings();
    let run = run_bin(
        &["sync", "--config-dir", dir.path().to_str().unwrap()],
        &[("WGS_WITHINGS_API_BASE", withings.base_url.as_str())],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("dry-run"), "stdout: {}", run.stdout);
}

#[test]
fn sync_apply_flag_is_accepted() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = empty_withings();
    let run = run_bin(
        &[
            "sync",
            "--apply",
            "--config-dir",
            dir.path().to_str().unwrap(),
            "--since",
            "2026-01-01",
            "--until",
            "2026-02-01",
        ],
        &[("WGS_WITHINGS_API_BASE", withings.base_url.as_str())],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("apply"), "stdout: {}", run.stdout);
    assert!(
        run.stdout.contains("2026-01-01..2026-02-01"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn base_url_overrides_flow_into_the_http_client() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let withings = empty_withings();
    let run = run_bin(
        &[
            "sync",
            "--verbose",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", "http://127.0.0.1:10002"),
            ("WGS_GARMIN_DIAUTH_BASE", "http://127.0.0.1:10003"),
            ("WGS_GARMIN_API_BASE", "http://127.0.0.1:10004"),
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stderr
            .contains(&format!("withings_api  = {}", withings.base_url)),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("garmin_sso    = http://127.0.0.1:10002"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("garmin_diauth = http://127.0.0.1:10003"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("garmin_api    = http://127.0.0.1:10004"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn default_config_dir_is_home_dot_config() {
    let home = TempDir::new();
    let run = run_bin(&["sync"], &[("HOME", home.path().to_str().unwrap())]);
    assert_eq!(run.code, 3);
    let expected = home.path().join(".config/withings-garmin-sync/config.toml");
    assert!(
        run.stderr.contains(&expected.display().to_string()),
        "expected {expected:?} in stderr: {}",
        run.stderr
    );
}
