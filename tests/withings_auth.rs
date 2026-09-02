//! Black-box tests for ticket 05: the Withings half of `auth` — the OAuth
//! authorization-code flow against a fake Withings token server, driven
//! entirely through the binary's stdin/stdout/exit-code seam.

mod common;

use common::{
    file_mode, form_value, parse_form, run_bin_stdin, write_file, FakeResponse, FakeServer, Route,
    TempDir,
};

fn write_config(dir: &TempDir) {
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n",
    );
}

fn withings_token_server(tokens: &'static str) -> FakeServer {
    FakeServer::start(vec![Route::post("/v2/oauth2", move |_req, _i| {
        FakeResponse::json(200, tokens)
    })])
}

/// The Withings token server points at the fake; all other hosts are dummy
/// values so a stray request to them would fail fast rather than hit prod.
fn auth_env(fake: &FakeServer) -> Vec<(&'static str, &str)> {
    vec![("WGS_WITHINGS_API_BASE", fake.base_url.as_str())]
}

const SUCCESS_TOKENS: &str = r#"{
  "status": 0,
  "body": {
    "userid": 4242,
    "access_token": "wa-access",
    "refresh_token": "wa-refresh",
    "expires_in": 10800,
    "scope": "user.metrics",
    "token_type": "Bearer"
  }
}"#;

#[test]
fn auth_prints_authorize_url_with_client_id_and_scope() {
    let dir = TempDir::new();
    write_config(&dir);
    let fake = withings_token_server(SUCCESS_TOKENS);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("http://localhost:8765/?code=abc123\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .contains("https://account.withings.com/oauth2_user/authorize2"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("response_type=code"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("client_id=test-client-id"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("scope=user.metrics"),
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("redirect_uri=http%3A%2F%2Flocalhost%3A8765%2F"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn auth_exchanges_pasted_code_and_persists_tokens() {
    let dir = TempDir::new();
    write_config(&dir);
    let fake = withings_token_server(SUCCESS_TOKENS);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("http://localhost:8765/?code=abc123&state=xyz\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // The exchange request must carry the exact Withings token-endpoint fields.
    let token_calls = fake.requests_for("POST", "/v2/oauth2");
    assert_eq!(token_calls.len(), 1, "calls: {:#?}", fake.requests());
    let fields = parse_form(&token_calls[0].body);
    assert_eq!(form_value(&fields, "action"), "requesttoken");
    assert_eq!(form_value(&fields, "grant_type"), "authorization_code");
    assert_eq!(form_value(&fields, "client_id"), "test-client-id");
    assert_eq!(form_value(&fields, "client_secret"), "test-client-secret");
    assert_eq!(form_value(&fields, "code"), "abc123");
    assert_eq!(
        form_value(&fields, "redirect_uri"),
        "http://localhost:8765/"
    );
    assert_eq!(fields.len(), 6, "unexpected extra fields: {fields:?}");

    // Tokens land in tokens.json with 0600 permissions.
    let tokens_path = dir.path().join("tokens.json");
    assert_eq!(file_mode(&tokens_path), 0o600);
    let tokens: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&tokens_path).unwrap()).unwrap();
    assert_eq!(tokens["withings"]["access_token"], "wa-access");
    assert_eq!(tokens["withings"]["refresh_token"], "wa-refresh");
    assert!(tokens["withings"]["expires_at"].is_u64());
}

#[test]
fn auth_accepts_a_bare_code_without_url() {
    let dir = TempDir::new();
    write_config(&dir);
    let fake = withings_token_server(SUCCESS_TOKENS);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("bare-code-123\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let token_calls = fake.requests_for("POST", "/v2/oauth2");
    let fields = parse_form(&token_calls[0].body);
    assert_eq!(form_value(&fields, "code"), "bare-code-123");
}

#[test]
fn auth_rejected_code_exits_4_without_writing_tokens() {
    let dir = TempDir::new();
    write_config(&dir);
    let fake = withings_token_server(r#"{"status": 2556}"#);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("http://localhost:8765/?code=bad-code\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("2556") || run.stderr.to_lowercase().contains("reject"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !dir.path().join("tokens.json").exists(),
        "tokens.json must not be written on failure"
    );
}

#[test]
fn auth_http_error_exits_4_without_writing_tokens() {
    let dir = TempDir::new();
    write_config(&dir);
    let fake = FakeServer::start(vec![Route::post("/v2/oauth2", |_req, _i| {
        FakeResponse::new(500, "boom")
    })]);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("http://localhost:8765/?code=abc123\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr.to_lowercase().contains("500"),
        "stderr: {}",
        run.stderr
    );
    assert!(!dir.path().join("tokens.json").exists());
}

#[test]
fn auth_prompts_for_missing_client_credentials_and_writes_config() {
    let dir = TempDir::new();
    let fake = withings_token_server(SUCCESS_TOKENS);

    // Feed client id, client secret, then the pasted redirect URL.
    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("my-client-id\nmy-client-secret\nhttp://localhost:8765/?code=abc123\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let config_path = dir.path().join("config.toml");
    assert_eq!(file_mode(&config_path), 0o600);
    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        config.contains(r#"client_id = "my-client-id""#),
        "config: {config}"
    );
    assert!(
        config.contains(r#"client_secret = "my-client-secret""#),
        "config: {config}"
    );
    let fields = parse_form(&fake.requests_for("POST", "/v2/oauth2")[0].body);
    assert_eq!(form_value(&fields, "client_id"), "my-client-id");
    assert_eq!(form_value(&fields, "client_secret"), "my-client-secret");
}

#[test]
fn auth_preserves_existing_garmin_tokens() {
    let dir = TempDir::new();
    write_config(&dir);
    write_file(
        &dir.path().join("tokens.json"),
        r#"{"garmin": {"access_token": "ga", "refresh_token": "gr", "client_id": "CID"}}"#,
    );
    let fake = withings_token_server(SUCCESS_TOKENS);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        Some("http://localhost:8765/?code=abc123\n"),
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let tokens: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("tokens.json")).unwrap())
            .unwrap();
    assert_eq!(tokens["garmin"]["access_token"], "ga");
    assert_eq!(tokens["garmin"]["refresh_token"], "gr");
    assert_eq!(tokens["garmin"]["client_id"], "CID");
    assert_eq!(tokens["withings"]["access_token"], "wa-access");
}

#[test]
fn auth_with_closed_stdin_fails_cleanly_without_writing_files() {
    let dir = TempDir::new();
    write_config(&dir);
    let fake = withings_token_server(SUCCESS_TOKENS);

    // No stdin: the redirect-code prompt hits EOF.
    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &auth_env(&fake),
        None,
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(!dir.path().join("tokens.json").exists());
}
