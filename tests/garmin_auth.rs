//! Black-box tests for ticket 06: the Garmin half of `auth` — the mobile-SSO
//! handshake with MFA and DI client-id retry, driven through the binary
//! against fake SSO/diauth servers.

mod common;

use common::{
    form_value, parse_form, run_bin_stdin, write_file, FakeResponse, FakeServer, Route, TempDir,
};
use serde_json::json;

use base64::Engine;

fn b64(input: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(input)
}

fn write_config(dir: &TempDir) {
    write_file(
        &dir.path().join("config.toml"),
        "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n",
    );
}

const WITHINGS_TOKENS: &str = r#"{
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

/// A successful DI token response for `access`/`refresh`.
fn di_tokens(access: &str, refresh: &str) -> String {
    json!({"access_token": access, "refresh_token": refresh}).to_string()
}

fn withings_server() -> FakeServer {
    FakeServer::start(vec![Route::post("/v2/oauth2", move |_req, _i| {
        FakeResponse::json(200, WITHINGS_TOKENS)
    })])
}

/// Run `auth` against fake Withings/SSO/diauth servers with the given stdin
/// (withings code, then garmin username, then garmin password, then any MFA
/// codes). Returns the run, the config dir (for file assertions), and the
/// servers.
fn run_garmin_auth(
    sso_routes: Vec<Route>,
    diauth_routes: Vec<Route>,
    stdin: &str,
) -> (common::Run, TempDir, FakeServer, FakeServer, FakeServer) {
    let dir = TempDir::new();
    write_config(&dir);
    let withings = withings_server();
    let sso = FakeServer::start(sso_routes);
    let diauth = FakeServer::start(diauth_routes);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", sso.base_url.as_str()),
            ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ],
        Some(stdin),
    );

    (run, dir, withings, sso, diauth)
}

fn read_tokens_json(dir: &TempDir) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.path().join("tokens.json")).unwrap()).unwrap()
}

fn run_garmin_auth_ok(
    sso_routes: Vec<Route>,
    diauth_routes: Vec<Route>,
    stdin: &str,
) -> (common::Run, TempDir, FakeServer, FakeServer) {
    let (run, dir, _withings, sso, diauth) = run_garmin_auth(sso_routes, diauth_routes, stdin);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    (run, dir, sso, diauth)
}

const SIGN_IN_QUERY: &str =
    "clientId=GCM_ANDROID_DARK&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fandroid";
const LOGIN_QUERY: &str = "clientId=GCM_ANDROID_DARK&locale=en-US&service=https%3A%2F%2Fmobile.integration.garmin.com%2Fgcm%2Fandroid";

fn login_success(ticket: &'static str) -> Route {
    Route::post("/mobile/api/login", move |_req, _i| {
        FakeResponse::json(
            200,
            &json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": ticket})
                .to_string(),
        )
    })
}

#[test]
fn auth_completes_mobile_sso_sequence_and_persists_di_tokens() {
    let (run, dir, sso, diauth) = run_garmin_auth_ok(
        vec![
            Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
                FakeResponse::new(200, "ok").with_header("Set-Cookie", "SESSIONID=abc123; Path=/")
            }),
            login_success("ST-12345"),
        ],
        vec![Route::post("/di-oauth2-service/oauth/token", |_req, _i| {
            FakeResponse::json(200, &di_tokens("di-access", "di-refresh"))
        })],
        "http://localhost:8765/?code=abc123\nuser@example.com\nhunter2\n",
    );

    // Ordered sequence: sign-in GET first, then login POST.
    let sso_requests = sso.requests();
    assert_eq!(sso_requests.len(), 2, "{sso_requests:#?}");
    assert_eq!(sso_requests[0].method, "GET");
    assert_eq!(sso_requests[0].path, "/mobile/sso/en_US/sign-in");
    assert_eq!(sso_requests[0].query, SIGN_IN_QUERY);
    assert_eq!(
        sso_requests[0].header("user-agent").unwrap(),
        "Mozilla/5.0 (Linux; Android 13; sdk_gphone64_arm64 Build/TE1A.220922.025; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/132.0.0.0 Mobile Safari/537.36"
    );

    // The login POST carries the session cookie and the exact JSON body.
    assert_eq!(sso_requests[1].method, "POST");
    assert_eq!(sso_requests[1].path, "/mobile/api/login");
    assert_eq!(sso_requests[1].query, LOGIN_QUERY);
    assert!(
        sso_requests[1]
            .header("cookie")
            .unwrap_or("")
            .contains("SESSIONID=abc123"),
        "login cookies: {:?}",
        sso_requests[1]
    );
    let body: serde_json::Value = serde_json::from_str(&sso_requests[1].body).unwrap();
    assert_eq!(
        body,
        json!({"username": "user@example.com", "password": "hunter2", "rememberMe": true, "captchaToken": ""})
    );

    // The service-ticket exchange at diauth: Basic auth + exact form + native
    // Android headers.
    let token_calls = diauth.requests_for("POST", "/di-oauth2-service/oauth/token");
    assert_eq!(token_calls.len(), 1);
    let first = &token_calls[0];
    assert_eq!(
        first.header("authorization").unwrap(),
        format!("Basic {}", b64("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2:"))
    );
    let fields = parse_form(&first.body);
    assert_eq!(
        form_value(&fields, "client_id"),
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
    );
    assert_eq!(form_value(&fields, "service_ticket"), "ST-12345");
    assert_eq!(
        form_value(&fields, "grant_type"),
        "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket"
    );
    assert_eq!(
        form_value(&fields, "service_url"),
        "https://mobile.integration.garmin.com/gcm/android"
    );
    assert_eq!(fields.len(), 4, "unexpected extra fields: {fields:?}");
    assert_eq!(first.header("user-agent").unwrap(), "GCM-Android-5.23");
    assert_eq!(
        first.header("x-garmin-user-agent").unwrap(),
        "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0"
    );
    assert_eq!(
        first.header("x-garmin-paired-app-version").unwrap(),
        "10861"
    );
    assert_eq!(first.header("x-garmin-client-platform").unwrap(), "Android");
    assert_eq!(first.header("x-app-ver").unwrap(), "10861");
    assert_eq!(first.header("x-lang").unwrap(), "en");
    assert_eq!(first.header("x-gcexperience").unwrap(), "GC5");
    assert_eq!(first.header("accept").unwrap(), "application/json");

    // Tokens (including the winning client id) are persisted.
    assert!(
        run.stdout.contains("auth: Garmin connected"),
        "stdout: {}",
        run.stdout
    );
    let tokens = read_tokens_json(&dir);
    assert_eq!(tokens["garmin"]["access_token"], "di-access");
    assert_eq!(tokens["garmin"]["refresh_token"], "di-refresh");
    assert_eq!(
        tokens["garmin"]["client_id"],
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
    );
}

#[test]
fn auth_persists_garmin_tokens_and_winning_client_id() {
    let (_run, dir, _sso, _diauth) = run_garmin_auth_ok(
        vec![
            Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
                FakeResponse::new(200, "ok")
            }),
            login_success("ST-999"),
        ],
        vec![Route::post("/di-oauth2-service/oauth/token", |_req, _i| {
            FakeResponse::json(200, &di_tokens("di-access-2", "di-refresh-2"))
        })],
        "http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n",
    );
    let tokens = read_tokens_json(&dir);
    assert_eq!(tokens["garmin"]["access_token"], "di-access-2");
    assert_eq!(tokens["garmin"]["refresh_token"], "di-refresh-2");
    assert_eq!(
        tokens["garmin"]["client_id"],
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
    );
    // The Withings tokens from the first half of `auth` sit alongside.
    assert_eq!(tokens["withings"]["access_token"], "wa-access");
}

#[test]
fn auth_mfa_branch_prompts_for_code_and_verifies() {
    let (run, _dir, sso, diauth) = run_garmin_auth_ok(
        vec![
            Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
                FakeResponse::new(200, "ok")
            }),
            Route::post("/mobile/api/login", move |_req, _i| {
                FakeResponse::json(
                    200,
                    &json!({
                        "responseStatus": {"type": "MFA_REQUIRED"},
                        "customerMfaInfo": {"mfaLastMethodUsed": "totp"}
                    })
                    .to_string(),
                )
            }),
            Route::post("/mobile/api/mfa/verifyCode", |_req, _i| {
                FakeResponse::json(
                    200,
                    &json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": "ST-MFA"})
                        .to_string(),
                )
            }),
        ],
        vec![Route::post("/di-oauth2-service/oauth/token", |_req, _i| {
            FakeResponse::json(200, &di_tokens("di-access", "di-refresh"))
        })],
        "http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n123456\n",
    );

    let verify_calls = sso.requests_for("POST", "/mobile/api/mfa/verifyCode");
    assert_eq!(verify_calls.len(), 1, "{verify_calls:#?}");
    assert_eq!(verify_calls[0].query, LOGIN_QUERY);
    let body: serde_json::Value = serde_json::from_str(&verify_calls[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "mfaMethod": "totp",
            "mfaVerificationCode": "123456",
            "rememberMyBrowser": true,
            "reconsentList": [],
            "mfaSetup": false
        })
    );

    let token_calls = diauth.requests_for("POST", "/di-oauth2-service/oauth/token");
    assert_eq!(
        form_value(&parse_form(&token_calls[0].body), "service_ticket"),
        "ST-MFA"
    );
    assert!(
        run.stdout.contains("auth: Garmin connected"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn auth_mfa_invalid_code_loops_until_valid() {
    let (_run, _dir, sso, diauth) = run_garmin_auth_ok(
        vec![
            Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
                FakeResponse::new(200, "ok")
            }),
            Route::post("/mobile/api/login", move |_req, _i| {
                FakeResponse::json(
                    200,
                    &json!({
                        "responseStatus": {"type": "MFA_REQUIRED"},
                        "customerMfaInfo": {"mfaLastMethodUsed": "email"}
                    })
                    .to_string(),
                )
            }),
            Route::post("/mobile/api/mfa/verifyCode", |_req, call| {
                if call == 0 {
                    FakeResponse::json(200, r#"{"responseStatus": {"type": "INVALID_MFA_CODE"}}"#)
                } else {
                    FakeResponse::json(
                        200,
                        &json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": "ST-OK"})
                            .to_string(),
                    )
                }
            }),
        ],
        vec![Route::post("/di-oauth2-service/oauth/token", |_req, _i| {
            FakeResponse::json(200, &di_tokens("di-access", "di-refresh"))
        })],
        "http://localhost:8765/?code=abc123\nuser@example.com\nsecret\nbadcode\ngoodcode\n",
    );

    let verify_calls = sso.requests_for("POST", "/mobile/api/mfa/verifyCode");
    assert_eq!(verify_calls.len(), 2, "{verify_calls:#?}");
    let first: serde_json::Value = serde_json::from_str(&verify_calls[0].body).unwrap();
    assert_eq!(first["mfaVerificationCode"], "badcode");
    assert_eq!(first["mfaMethod"], "email");
    let second: serde_json::Value = serde_json::from_str(&verify_calls[1].body).unwrap();
    assert_eq!(second["mfaVerificationCode"], "goodcode");

    let token_calls = diauth.requests_for("POST", "/di-oauth2-service/oauth/token");
    assert_eq!(
        form_value(&parse_form(&token_calls[0].body), "service_ticket"),
        "ST-OK"
    );
}

#[test]
fn auth_retries_di_client_ids_in_order() {
    let (_run, dir, _sso, diauth) = run_garmin_auth_ok(
        vec![
            Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
                FakeResponse::new(200, "ok")
            }),
            login_success("ST-777"),
        ],
        vec![Route::post(
            "/di-oauth2-service/oauth/token",
            |_req, call| {
                if call == 0 {
                    FakeResponse::new(401, r#"{"error": "invalid_client"}"#)
                } else {
                    FakeResponse::json(200, &di_tokens("di-access", "di-refresh"))
                }
            },
        )],
        "http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n",
    );

    let token_calls = diauth.requests_for("POST", "/di-oauth2-service/oauth/token");
    assert_eq!(token_calls.len(), 2, "{token_calls:#?}");
    assert_eq!(
        token_calls[0].header("authorization").unwrap(),
        format!("Basic {}", b64("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2:"))
    );
    assert_eq!(
        token_calls[1].header("authorization").unwrap(),
        format!("Basic {}", b64("GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4:"))
    );
    assert_eq!(
        form_value(&parse_form(&token_calls[1].body), "client_id"),
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4"
    );
    assert_eq!(
        read_tokens_json(&dir)["garmin"]["client_id"],
        "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4"
    );
}

#[test]
fn auth_invalid_username_password_exits_4_without_tokens() {
    let dir = TempDir::new();
    write_config(&dir);
    let withings = withings_server();
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
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", sso.base_url.as_str()),
            ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ],
        Some("http://localhost:8765/?code=abc123\nuser@example.com\nwrongpass\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr
            .to_lowercase()
            .contains("invalid username or password"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !dir.path().join("tokens.json").exists(),
        "no tokens must be persisted on a failed login"
    );
}

#[test]
fn auth_di_exchange_total_failure_exits_4() {
    let dir = TempDir::new();
    write_config(&dir);
    let withings = withings_server();
    let sso = FakeServer::start(vec![
        Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
            FakeResponse::new(200, "ok")
        }),
        login_success("ST-888"),
    ]);
    let diauth = FakeServer::start(vec![Route::post(
        "/di-oauth2-service/oauth/token",
        |_req, _i| FakeResponse::new(401, "nope"),
    )]);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", sso.base_url.as_str()),
            ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ],
        Some("http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    // All three candidates were tried.
    assert_eq!(
        diauth
            .requests_for("POST", "/di-oauth2-service/oauth/token")
            .len(),
        3
    );
    assert!(!dir.path().join("tokens.json").exists());
}

#[test]
fn auth_garmin_429_exits_4_with_rate_limit_message() {
    let dir = TempDir::new();
    write_config(&dir);
    let withings = withings_server();
    let sso = FakeServer::start(vec![
        Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
            FakeResponse::new(200, "ok")
        }),
        Route::post("/mobile/api/login", |_req, _i| {
            FakeResponse::new(429, "slow down")
        }),
    ]);
    let diauth = FakeServer::start(vec![]);

    let run = run_bin_stdin(
        &["auth", "--config-dir", dir.path().to_str().unwrap()],
        &[
            ("WGS_WITHINGS_API_BASE", withings.base_url.as_str()),
            ("WGS_GARMIN_SSO_BASE", sso.base_url.as_str()),
            ("WGS_GARMIN_DIAUTH_BASE", diauth.base_url.as_str()),
        ],
        Some("http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n"),
    );

    assert_eq!(run.code, 4, "stdout: {}", run.stdout);
    assert!(
        run.stderr.to_lowercase().contains("rate") || run.stderr.contains("429"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn auth_uses_encoded_service_in_login_query() {
    // Already covered in the sequence test via SIGN_IN_QUERY/LOGIN_QUERY; this
    // test pins the exact query strings so a drift in encoding is caught.
    let (_run, _dir, sso, _diauth) = run_garmin_auth_ok(
        vec![
            Route::get("/mobile/sso/en_US/sign-in", |_req, _i| {
                FakeResponse::new(200, "ok")
            }),
            login_success("ST-Q"),
        ],
        vec![Route::post("/di-oauth2-service/oauth/token", |_req, _i| {
            FakeResponse::json(200, &di_tokens("a", "b"))
        })],
        "http://localhost:8765/?code=abc123\nuser@example.com\nsecret\n",
    );
    let requests = sso.requests();
    assert_eq!(requests[0].query, SIGN_IN_QUERY);
    assert_eq!(requests[1].query, LOGIN_QUERY);
}
