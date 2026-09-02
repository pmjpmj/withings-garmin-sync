//! Black-box tests for the walking-skeleton CLI.
//!
//! These drive the compiled binary against a temp config dir and assert only on
//! observable behavior: stdout/stderr text, exit codes, and the files the
//! binary writes (including their permissions). No internals are reached into.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_withings-garmin-sync");

const WGS_VARS: [&str; 4] = [
    "WGS_WITHINGS_API_BASE",
    "WGS_GARMIN_SSO_BASE",
    "WGS_GARMIN_DIAUTH_BASE",
    "WGS_GARMIN_API_BASE",
];

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let mut n = COUNTER.fetch_add(1, Ordering::SeqCst);
        loop {
            let path =
                std::env::temp_dir().join(format!("wgs-cli-test-{}-{}", std::process::id(), n));
            if !path.exists() {
                fs::create_dir_all(&path).expect("create temp dir");
                return TempDir(path);
            }
            n = COUNTER.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run the binary with `args`, isolating it from the real HOME and any ambient
/// base-URL overrides. `envs` is applied last so a test can set its own HOME.
fn run_bin(args: &[&str], envs: &[(&str, &str)]) -> Run {
    let home = TempDir::new();
    let mut cmd = Command::new(BIN);
    cmd.args(args);
    cmd.env("HOME", home.path());
    for var in WGS_VARS {
        cmd.env_remove(var);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let Output {
        status,
        stdout,
        stderr,
    } = cmd.output().expect("run binary");
    Run {
        code: status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

const VALID_CONFIG: &str =
    "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n";
const VALID_TOKENS: &str = r#"{
  "withings": { "access_token": "wa", "refresh_token": "wr", "expires_at": "2026-01-01T00:00:00Z" },
  "garmin": { "access_token": "ga", "refresh_token": "gr", "client_id": "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2" }
}"#;

fn write_valid_config_and_tokens(dir: &Path) {
    write_file(&dir.join("config.toml"), VALID_CONFIG);
    write_file(&dir.join("tokens.json"), VALID_TOKENS);
}

fn file_mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

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

#[test]
fn auth_writes_config_and_tokens_with_0600() {
    let dir = TempDir::new();
    let run = run_bin(&["auth", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("auth: wrote"), "stdout: {}", run.stdout);

    let config = dir.path().join("config.toml");
    let tokens = dir.path().join("tokens.json");
    assert!(config.exists());
    assert!(tokens.exists());
    assert_eq!(file_mode(&config), 0o600);
    assert_eq!(file_mode(&tokens), 0o600);
}

#[test]
fn sync_valid_config_dry_run_succeeds_by_default() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
    let run = run_bin(&["sync", "--config-dir", dir.path().to_str().unwrap()], &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("dry-run"), "stdout: {}", run.stdout);
}

#[test]
fn sync_apply_flag_is_accepted() {
    let dir = TempDir::new();
    write_valid_config_and_tokens(dir.path());
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
        &[],
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
    let run = run_bin(
        &[
            "sync",
            "--verbose",
            "--config-dir",
            dir.path().to_str().unwrap(),
        ],
        &[
            ("WGS_WITHINGS_API_BASE", "http://127.0.0.1:10001"),
            ("WGS_GARMIN_SSO_BASE", "http://127.0.0.1:10002"),
            ("WGS_GARMIN_DIAUTH_BASE", "http://127.0.0.1:10003"),
            ("WGS_GARMIN_API_BASE", "http://127.0.0.1:10004"),
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stderr
            .contains("withings_api  = http://127.0.0.1:10001"),
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
