//! Shared black-box test helpers: temp dirs, binary runner, and an in-process
//! fake HTTP server. Tests observe external behavior only — the binary's
//! stdout/stderr, exit codes, files it writes, and the requests the fake
//! servers receive.
//!
//! Each test crate uses a subset of these helpers, so dead-code warnings are
//! expected across crates.
#![allow(dead_code)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const BIN: &str = env!("CARGO_BIN_EXE_withings-garmin-sync");

pub const WGS_VARS: [&str; 4] = [
    "WGS_WITHINGS_API_BASE",
    "WGS_GARMIN_SSO_BASE",
    "WGS_GARMIN_DIAUTH_BASE",
    "WGS_GARMIN_API_BASE",
];

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let mut n = COUNTER.fetch_add(1, Ordering::SeqCst);
        loop {
            let path = std::env::temp_dir().join(format!("wgs-test-{}-{}", std::process::id(), n));
            if !path.exists() {
                fs::create_dir_all(&path).expect("create temp dir");
                return TempDir(path);
            }
            n = COUNTER.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run the binary with `args`, isolating it from the real HOME and any ambient
/// base-URL overrides. `envs` is applied last so a test can set its own HOME.
pub fn run_bin(args: &[&str], envs: &[(&str, &str)]) -> Run {
    run_bin_stdin(args, envs, None)
}

/// Like [`run_bin`], but feeds `stdin` to the binary's standard input.
pub fn run_bin_stdin(args: &[&str], envs: &[(&str, &str)], stdin: Option<&str>) -> Run {
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
    match stdin {
        Some(input) => {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn binary");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(input.as_bytes())
                .expect("write stdin");
            let output = child.wait_with_output().expect("wait for binary");
            Run::from(output)
        }
        None => Run::from(cmd.output().expect("run binary")),
    }
}

impl Run {
    fn from(output: Output) -> Self {
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

pub fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

pub fn file_mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

pub const VALID_CONFIG: &str =
    "[withings]\nclient_id = \"test-client-id\"\nclient_secret = \"test-client-secret\"\n";
pub const VALID_TOKENS: &str = r#"{
  "withings": { "access_token": "wa", "refresh_token": "wr", "expires_at": 4102444800 },
  "garmin": { "access_token": "ga", "refresh_token": "gr", "client_id": "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2" }
}"#;

pub fn write_valid_config_and_tokens(dir: &Path) {
    write_file(&dir.join("config.toml"), VALID_CONFIG);
    write_file(&dir.join("tokens.json"), VALID_TOKENS);
}

/// Parse an `application/x-www-form-urlencoded` body into `(key, value)` pairs,
/// URL-decoding each component.
pub fn parse_form(body: &str) -> Vec<(String, String)> {
    fn decode(s: &str) -> String {
        let mut out = Vec::with_capacity(s.len());
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b'%' if i + 2 < bytes.len() => {
                    let hex = &s[i + 1..i + 3];
                    if let Ok(v) = u8::from_str_radix(hex, 16) {
                        out.push(v);
                    }
                    i += 3;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    body.split('&')
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once('=') {
            Some((key, value)) => (decode(key), decode(value)),
            None => (decode(part), String::new()),
        })
        .collect()
}

/// Find the value for `key` in a parsed form; panics with a helpful message if
/// absent.
pub fn form_value<'a>(pairs: &'a [(String, String)], key: &str) -> &'a str {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("form field {key:?} missing from {pairs:?}"))
}

// ---------------------------------------------------------------------------
// Fake HTTP server
// ---------------------------------------------------------------------------

/// One request the fake server received.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    /// Path without the query string, e.g. `/mobile/api/login`.
    pub path: String,
    /// Raw query string (without `?`), e.g. `clientId=GCM_ANDROID_DARK`.
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A canned response the fake server sends.
#[derive(Debug, Clone)]
pub struct FakeResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl FakeResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        FakeResponse {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn json(status: u16, body: &str) -> Self {
        FakeResponse {
            status,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

type RouteHandler = Box<dyn Fn(&RecordedRequest, usize) -> FakeResponse + Send + Sync>;

pub struct Route {
    method: &'static str,
    path: &'static str,
    prefix: bool,
    handler: RouteHandler,
}

impl Route {
    pub fn new(
        method: &'static str,
        path: &'static str,
        handler: impl Fn(&RecordedRequest, usize) -> FakeResponse + Send + Sync + 'static,
    ) -> Self {
        Route {
            method,
            path,
            prefix: false,
            handler: Box::new(handler),
        }
    }

    pub fn post(
        path: &'static str,
        handler: impl Fn(&RecordedRequest, usize) -> FakeResponse + Send + Sync + 'static,
    ) -> Self {
        Route::new("POST", path, handler)
    }

    pub fn get(
        path: &'static str,
        handler: impl Fn(&RecordedRequest, usize) -> FakeResponse + Send + Sync + 'static,
    ) -> Self {
        Route::new("GET", path, handler)
    }

    /// A GET route that matches any path starting with `prefix` (used for
    /// endpoints whose path embeds dynamic dates).
    pub fn get_prefix(
        prefix: &'static str,
        handler: impl Fn(&RecordedRequest, usize) -> FakeResponse + Send + Sync + 'static,
    ) -> Self {
        Route {
            method: "GET",
            path: prefix,
            prefix: true,
            handler: Box::new(handler),
        }
    }
}

pub struct FakeServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Start a server with the given routes. Unmatched requests get a 404.
    pub fn start(routes: Vec<Route>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{addr}");

        let requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let reqs = Arc::clone(&requests);
        let stop_flag = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let routes = &routes;
                        let _ = handle_connection(stream, routes, &reqs);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    // A client can abort a queued connection under parallel
                    // load (ECONNABORTED/ECONNRESET); that must not kill the
                    // accept loop. Interrupted accepts likewise continue.
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::ConnectionAborted
                            || e.kind() == std::io::ErrorKind::ConnectionReset
                            || e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });

        FakeServer {
            base_url,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    /// All requests received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Requests matching `method` and `path`, in arrival order.
    pub fn requests_for(&self, method: &str, path: &str) -> Vec<RecordedRequest> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == method && r.path == path)
            .collect()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop: connect once so a nonblocking accept sees us.
        let _ = TcpStream::connect(self.base_url.trim_start_matches("http://"));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    routes: &[Route],
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) -> std::io::Result<()> {
    // On macOS, sockets accepted from a nonblocking listener inherit the
    // nonblocking flag; put them back in blocking mode (bounded by the read
    // timeout) so reads wait for the client instead of EAGAIN-ing.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    let request = read_request(&mut stream)?;
    let recorded = request.clone();

    let calls = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == request.method && r.path == request.path)
        .count();
    requests.lock().unwrap().push(recorded);

    let response = routes
        .iter()
        .find(|route| {
            route.method.eq_ignore_ascii_case(&request.method)
                && if route.prefix {
                    request.path.starts_with(route.path)
                } else {
                    route.path == request.path
                }
        })
        .map(|route| (route.handler)(&request, calls))
        .unwrap_or_else(|| FakeResponse::new(404, "not found"));

    write_response(&mut stream, &response)
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<RecordedRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before headers",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("");
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.to_string(), String::new()),
    };

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    let body_start = header_end + 4;
    let mut body_bytes = buf[body_start..].to_vec();
    while body_bytes.len() < content_length {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        body_bytes.extend_from_slice(&tmp[..n]);
    }
    if body_bytes.len() > content_length {
        body_bytes.truncate(content_length);
    }

    Ok(RecordedRequest {
        method,
        path,
        query,
        headers,
        body: String::from_utf8_lossy(&body_bytes).into_owned(),
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_response(stream: &mut TcpStream, response: &FakeResponse) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        412 => "Precondition Failed",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Response",
    };
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, reason);
    head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    head.push_str("Connection: close\r\n");
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}
