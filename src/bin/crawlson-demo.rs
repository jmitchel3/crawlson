use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use serde::Serialize;

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_FORM_BODY_BYTES: usize = 512;
const MAX_FIXTURE_NAME_BYTES: usize = 64;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const FIXTURE_TTL: Duration = Duration::from_secs(10 * 60);

const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Crawlson demo</title>
  <style>
    * { box-sizing: border-box; }
    html { color-scheme: light; }
    body {
      margin: 0;
      min-height: 100vh;
      background: #f4f7fb;
      color: #172033;
      font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    main {
      width: min(760px, calc(100% - 48px));
      margin: 72px auto;
      padding: 56px;
      border: 1px solid #dce3ee;
      border-radius: 18px;
      background: #ffffff;
      box-shadow: 0 18px 50px rgb(32 49 78 / 12%);
    }
    .eyebrow {
      margin: 0 0 12px;
      color: #365b96;
      font-size: 14px;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }
    h1 {
      margin: 0;
      font-size: clamp(34px, 5vw, 52px);
      line-height: 1.08;
      letter-spacing: -0.035em;
    }
    #journey-status {
      max-width: 590px;
      margin: 24px 0 0;
      color: #52617a;
      font-size: 19px;
      line-height: 1.6;
    }
    .action-button {
      display: inline-flex;
      min-height: 54px;
      align-items: center;
      justify-content: center;
      margin-top: 36px;
      padding: 14px 28px;
      border: 0;
      border-radius: 10px;
      background: #2157d5;
      color: #ffffff;
      font: inherit;
      font-size: 18px;
      font-weight: 750;
      cursor: default;
      text-decoration: none;
    }
    #redirect-button {
      margin-left: 12px;
      background: #59647a;
    }
  </style>
</head>
<body>
  <main aria-labelledby="welcome-heading">
    <p class="eyebrow">Safe local fixture</p>
    <h1 id="welcome-heading">Welcome to the Crawlson demo</h1>
    <p id="journey-status">This read-only page proves the complete journey, evidence, finding, and guide loop without credentials or third-party services.</p>
    <a class="action-button" id="action-button" href="/complete">Continue</a>
    <a class="action-button" id="redirect-button" href="/redirect">Broken redirect fixture</a>
  </main>
</body>
</html>
"#;

const COMPLETE_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Crawlson demo complete</title>
  <style>
    * { box-sizing: border-box; }
    html { color-scheme: light; }
    body {
      margin: 0;
      min-height: 100vh;
      background: #f4f7fb;
      color: #172033;
      font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    main {
      width: min(760px, calc(100% - 48px));
      margin: 72px auto;
      padding: 56px;
      border: 1px solid #dce3ee;
      border-radius: 18px;
      background: #ffffff;
      box-shadow: 0 18px 50px rgb(32 49 78 / 12%);
    }
    .eyebrow {
      margin: 0 0 12px;
      color: #365b96;
      font-size: 14px;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }
    h1 {
      margin: 0;
      font-size: clamp(34px, 5vw, 52px);
      line-height: 1.08;
      letter-spacing: -0.035em;
    }
    #completion-status {
      max-width: 590px;
      margin: 24px 0 0;
      color: #52617a;
      font-size: 19px;
      line-height: 1.6;
    }
  </style>
</head>
<body>
  <main aria-labelledby="completion-heading">
    <p class="eyebrow">Verified destination</p>
    <h1 id="completion-heading">Journey complete</h1>
    <p id="completion-status">The same-origin Continue link reached its deterministic destination.</p>
  </main>
</body>
</html>
"#;

const AUTHENTICATED_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Crawlson authenticated demo</title>
  <style>
    * { box-sizing: border-box; }
    html { color-scheme: light; }
    body { margin: 0; min-height: 100vh; background: #f4f7fb; color: #172033; font-family: ui-sans-serif, system-ui, sans-serif; }
    main { width: min(760px, calc(100% - 48px)); margin: 72px auto; padding: 56px; border: 1px solid #dce3ee; border-radius: 18px; background: #fff; box-shadow: 0 18px 50px rgb(32 49 78 / 12%); }
    .eyebrow { margin: 0 0 12px; color: #365b96; font-size: 14px; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
    h1 { margin: 0; font-size: clamp(34px, 5vw, 52px); line-height: 1.08; letter-spacing: -.035em; }
    #authenticated-role { display: inline-flex; margin-top: 28px; padding: 14px 20px; border-radius: 10px; background: #e6f5ec; color: #155b35; font-size: 19px; font-weight: 750; }
  </style>
</head>
<body>
  <main aria-labelledby="authenticated-heading">
    <p class="eyebrow">Disposable local session</p>
    <h1 id="authenticated-heading">Authenticated demo</h1>
    <p id="authenticated-role">Sign in required</p>
  </main>
  <script nonce="crawlson-demo">
    (() => {
      const state = window.localStorage.getItem("crawlson_demo_session");
      if (state && state.startsWith("crawlson-demo-fixture-")) {
        document.getElementById("authenticated-role").textContent = "Viewer access";
      }
    })();
  </script>
</body>
</html>
"#;

const MUTATION_EMPTY_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Crawlson disposable fixture</title>
  <style>
    * { box-sizing: border-box; }
    html { color-scheme: light; }
    body { margin: 0; min-height: 100vh; background: #f4f7fb; color: #172033; font-family: ui-sans-serif, system-ui, sans-serif; }
    main { width: min(760px, calc(100% - 48px)); margin: 72px auto; padding: 56px; border: 1px solid #dce3ee; border-radius: 18px; background: #fff; box-shadow: 0 18px 50px rgb(32 49 78 / 12%); }
    .eyebrow { margin: 0 0 12px; color: #365b96; font-size: 14px; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
    h1 { margin: 0; font-size: clamp(34px, 5vw, 52px); line-height: 1.08; letter-spacing: -.035em; }
    #fixture-empty { margin: 24px 0 0; color: #52617a; font-size: 19px; line-height: 1.6; }
    #network-guard-probes { margin: 12px 0 0; color: #52617a; font-size: 14px; line-height: 1.5; }
    label { display: block; margin-top: 30px; font-size: 16px; font-weight: 750; }
    input { width: 100%; min-height: 50px; margin-top: 9px; padding: 12px 14px; border: 1px solid #9eabc0; border-radius: 8px; color: #172033; font: inherit; }
    button { min-height: 52px; margin-top: 22px; padding: 13px 24px; border: 0; border-radius: 9px; background: #2157d5; color: #fff; font: inherit; font-size: 17px; font-weight: 750; }
  </style>
</head>
<body>
  <main aria-labelledby="fixture-heading">
    <p class="eyebrow">Disposable local fixture</p>
    <h1 id="fixture-heading">Create a fixture</h1>
    <p id="fixture-empty">Disposable fixture absent.</p>
    <p id="network-guard-probes">Network guard probes idle.</p>
    <form id="fixture-form" action="/mutation/create" method="post">
      <label for="fixture-name">Public fixture token</label>
      <input id="fixture-name" name="fixture_name" type="text" maxlength="64" autocomplete="off" required>
      <button id="create-fixture-button" type="submit">Create fixture</button>
    </form>
  </main>
  <script nonce="crawlson-demo">
    (() => {
      const value = window.localStorage.getItem("crawlson_demo_network_trap_origin");
      if (!value) {
        return;
      }
      let trap;
      try {
        trap = new URL(value);
      } catch {
        return;
      }
      if (trap.protocol !== "http:" || trap.hostname !== "127.0.0.1" || !trap.port || trap.pathname !== "/") {
        return;
      }
      const endpoint = (path) => new URL(path, trap).href;
      try {
        void fetch(endpoint("fetch"), { method: "POST", mode: "no-cors", body: "probe" });
      } catch {}
      try {
        navigator.sendBeacon(endpoint("beacon"), "probe");
      } catch {}
      try {
        const websocket = new WebSocket(endpoint("websocket").replace(/^http:/, "ws:"));
        websocket.addEventListener("open", () => websocket.close(), { once: true });
      } catch {}
      document.getElementById("network-guard-probes").textContent =
        "fetch, sendBeacon, and WebSocket probes dispatched.";
    })();
  </script>
</body>
</html>
"#;

const MUTATION_CREATED_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Crawlson disposable fixture created</title>
  <style>
    * { box-sizing: border-box; }
    html { color-scheme: light; }
    body { margin: 0; min-height: 100vh; background: #f4f7fb; color: #172033; font-family: ui-sans-serif, system-ui, sans-serif; }
    main { width: min(760px, calc(100% - 48px)); margin: 72px auto; padding: 56px; border: 1px solid #dce3ee; border-radius: 18px; background: #fff; box-shadow: 0 18px 50px rgb(32 49 78 / 12%); }
    .eyebrow { margin: 0 0 12px; color: #365b96; font-size: 14px; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
    h1 { margin: 0; font-size: clamp(34px, 5vw, 52px); line-height: 1.08; letter-spacing: -.035em; }
    #fixture-result { margin: 24px 0 0; color: #155b35; font-size: 19px; font-weight: 750; line-height: 1.6; }
    button { min-height: 52px; margin-top: 30px; padding: 13px 24px; border: 0; border-radius: 9px; background: #a52b2b; color: #fff; font: inherit; font-size: 17px; font-weight: 750; }
  </style>
</head>
<body>
  <main aria-labelledby="fixture-created-heading">
    <p class="eyebrow">Disposable local fixture</p>
    <h1 id="fixture-created-heading">Fixture ready</h1>
    <p id="fixture-result">Disposable fixture created.</p>
    <form id="cleanup-form" action="/mutation/delete" method="post">
      <button id="remove-fixture-button" type="submit">Remove fixture</button>
    </form>
  </main>
</body>
</html>
"#;

#[derive(Debug)]
struct StoredFixture {
    _name: String,
    expires_at: Instant,
}

#[derive(Debug)]
struct FixtureStore {
    fixture: Option<StoredFixture>,
    ttl: Duration,
}

impl FixtureStore {
    fn new(ttl: Duration) -> Self {
        Self { fixture: None, ttl }
    }

    fn expire(&mut self, now: Instant) {
        if self
            .fixture
            .as_ref()
            .is_some_and(|fixture| now >= fixture.expires_at)
        {
            self.fixture = None;
        }
    }

    fn is_active(&mut self, now: Instant) -> bool {
        self.expire(now);
        self.fixture.is_some()
    }

    fn create(&mut self, name: String, now: Instant) -> Result<(), ()> {
        self.expire(now);
        if self.fixture.is_some() {
            return Err(());
        }
        self.fixture = Some(StoredFixture {
            _name: name,
            expires_at: now + self.ttl,
        });
        Ok(())
    }

    fn clear(&mut self) {
        self.fixture = None;
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "crawlson-demo",
    version,
    about = "Run Crawlson's credential-free, loopback-only demo application"
)]
struct Arguments {
    /// Loopback address to bind. Non-loopback addresses are rejected.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    bind: IpAddr,

    /// TCP port to bind. Use 0 to request an ephemeral operating-system port.
    #[arg(long, default_value_t = 4173)]
    port: u16,

    /// Emit exactly one machine-readable readiness object on stdout.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
struct Readiness<'a> {
    schema_version: u8,
    status: &'a str,
    origin: String,
    pid: u32,
}

fn main() -> ExitCode {
    match run(Arguments::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("crawlson-demo: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Arguments) -> Result<(), String> {
    validate_bind(arguments.bind)?;
    let listener = TcpListener::bind(SocketAddr::new(arguments.bind, arguments.port))
        .map_err(|error| format!("could not bind the loopback demo server: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure the demo listener: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("could not inspect the demo listener: {error}"))?;
    let origin = format!("http://{address}");

    let stopping = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&stopping);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Release))
        .map_err(|error| format!("could not install the shutdown handler: {error}"))?;

    let readiness = Readiness {
        schema_version: 1,
        status: "ready",
        origin: origin.clone(),
        pid: std::process::id(),
    };
    if arguments.json {
        println!(
            "{}",
            serde_json::to_string(&readiness)
                .map_err(|error| format!("could not serialize readiness: {error}"))?
        );
    } else {
        println!("Crawlson demo ready at {origin}");
    }
    std::io::stdout()
        .flush()
        .map_err(|error| format!("could not emit readiness: {error}"))?;

    let mut fixtures = FixtureStore::new(FIXTURE_TTL);
    while !stopping.load(Ordering::Acquire) {
        fixtures.expire(Instant::now());
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = serve(stream, &mut fixtures) {
                    eprintln!("crawlson-demo: request failed: {error}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("could not accept a demo request: {error}")),
        }
    }
    Ok(())
}

fn validate_bind(address: IpAddr) -> Result<(), String> {
    if address.is_loopback() {
        Ok(())
    } else {
        Err("--bind must be an IPv4 or IPv6 loopback address".to_owned())
    }
}

fn serve(mut stream: TcpStream, fixtures: &mut FixtureStore) -> Result<(), String> {
    // Windows can propagate the listener's nonblocking mode to accepted streams.
    // Request handling is deliberately synchronous, so normalize every stream.
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("could not configure the request stream: {error}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| error.to_string())?;
    let mut request = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    let mut expected_request_bytes = None;
    loop {
        let count = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > MAX_REQUEST_BYTES {
            write_response(
                &mut stream,
                Response::empty(431, "Request Header Fields Too Large"),
            )?;
            return Ok(());
        }
        if let Some(expected) = expected_request_bytes {
            if request.len() >= expected {
                break;
            }
            continue;
        }
        if let Some(header_end) = find_header_end(&request) {
            match expected_request_size(&request[..header_end]) {
                Ok(expected) if expected <= MAX_REQUEST_BYTES => {
                    expected_request_bytes = Some(expected);
                    if request.len() >= expected {
                        break;
                    }
                }
                Ok(_) => {
                    write_response(&mut stream, Response::empty(413, "Content Too Large"))?;
                    return Ok(());
                }
                Err(response) => {
                    write_response(&mut stream, response)?;
                    return Ok(());
                }
            }
        } else if request.len() > MAX_HEADER_BYTES {
            write_response(
                &mut stream,
                Response::empty(431, "Request Header Fields Too Large"),
            )?;
            return Ok(());
        }
    }
    if expected_request_bytes.is_none() {
        write_response(&mut stream, Response::empty(400, "Bad Request"))?;
        return Ok(());
    }
    if request.len() != expected_request_bytes.expect("checked above") {
        write_response(&mut stream, Response::empty(400, "Bad Request"))?;
        return Ok(());
    }
    let response = route(&request, fixtures);
    write_response(&mut stream, response)
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn expected_request_size(headers: &[u8]) -> Result<usize, Response> {
    let headers = std::str::from_utf8(headers).map_err(|_| Response::empty(400, "Bad Request"))?;
    let mut content_length = None;
    for line in headers
        .split("\r\n")
        .skip(1)
        .filter(|line| !line.is_empty())
    {
        let Some((name, value)) = line.split_once(':') else {
            return Err(Response::empty(400, "Bad Request"));
        };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(Response::empty(400, "Bad Request"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(Response::empty(400, "Bad Request"));
            }
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| Response::empty(400, "Bad Request"))?;
            if length > MAX_FORM_BODY_BYTES {
                return Err(Response::empty(413, "Content Too Large"));
            }
            content_length = Some(length);
        }
    }
    headers
        .len()
        .checked_add(content_length.unwrap_or(0))
        .ok_or_else(|| Response::empty(413, "Content Too Large"))
}

#[derive(Debug)]
struct Request<'a> {
    method: &'a str,
    path: &'a str,
    content_type: Option<&'a str>,
    body: &'a [u8],
}

fn parse_request(request: &[u8]) -> Result<Request<'_>, Response> {
    if request.len() > MAX_REQUEST_BYTES {
        return Err(Response::empty(413, "Content Too Large"));
    }
    let Some(header_end) = find_header_end(request) else {
        return Err(Response::empty(400, "Bad Request"));
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(Response::empty(431, "Request Header Fields Too Large"));
    }
    let headers = std::str::from_utf8(&request[..header_end])
        .map_err(|_| Response::empty(400, "Bad Request"))?;
    let mut lines = headers.split("\r\n");
    let Some(line) = lines.next() else {
        return Err(Response::empty(400, "Bad Request"));
    };
    let mut parts = line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Response::empty(400, "Bad Request"));
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || !target.starts_with('/') {
        return Err(Response::empty(400, "Bad Request"));
    }

    let mut content_length = None;
    let mut content_type = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(Response::empty(400, "Bad Request"));
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(Response::empty(400, "Bad Request"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(Response::empty(400, "Bad Request"));
            }
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| Response::empty(400, "Bad Request"))?,
            );
        }
        if name.eq_ignore_ascii_case("content-type") {
            if content_type.is_some() {
                return Err(Response::empty(400, "Bad Request"));
            }
            content_type = Some(value);
        }
    }

    let body = &request[header_end..];
    let declared_length = content_length.unwrap_or(0);
    if declared_length > MAX_FORM_BODY_BYTES {
        return Err(Response::empty(413, "Content Too Large"));
    }
    if body.len() != declared_length {
        return Err(Response::empty(400, "Bad Request"));
    }
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    Ok(Request {
        method,
        path,
        content_type,
        body,
    })
}

fn is_form_content_type(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

fn decode_fixture_name(body: &[u8]) -> Result<String, ()> {
    let body = std::str::from_utf8(body).map_err(|_| ())?;
    let mut pairs = body.split('&');
    let pair = pairs.next().ok_or(())?;
    if pairs.next().is_some() {
        return Err(());
    }
    let (name, encoded_value) = pair.split_once('=').ok_or(())?;
    if name != "fixture_name" {
        return Err(());
    }
    let mut decoded = Vec::with_capacity(encoded_value.len());
    let bytes = encoded_value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = decode_hex(bytes[index + 1]).ok_or(())?;
                let low = decode_hex(bytes[index + 2]).ok_or(())?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'%' | b'+' => return Err(()),
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
        if decoded.len() > MAX_FIXTURE_NAME_BYTES {
            break;
        }
    }
    if decoded.is_empty()
        || decoded.len() > MAX_FIXTURE_NAME_BYTES
        || !decoded
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(());
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug)]
struct Response {
    status: u16,
    reason: &'static str,
    media_type: &'static str,
    body: &'static [u8],
    include_body: bool,
    allow: Option<&'static str>,
    location: Option<&'static str>,
    connect_src: Option<&'static str>,
}

impl Response {
    fn empty(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: None,
            connect_src: None,
        }
    }
}

fn route(request: &[u8], fixtures: &mut FixtureStore) -> Response {
    route_at(request, fixtures, Instant::now())
}

fn route_at(request: &[u8], fixtures: &mut FixtureStore, now: Instant) -> Response {
    fixtures.expire(now);
    let request = match parse_request(request) {
        Ok(request) => request,
        Err(response) => return response,
    };
    match (request.method, request.path) {
        ("GET" | "HEAD", "/") => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: PAGE.as_bytes(),
            include_body: request.method == "GET",
            allow: None,
            location: None,
            connect_src: None,
        },
        ("GET" | "HEAD", "/complete") => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: COMPLETE_PAGE.as_bytes(),
            include_body: request.method == "GET",
            allow: None,
            location: None,
            connect_src: None,
        },
        ("GET" | "HEAD", "/redirect") => Response {
            status: 302,
            reason: "Found",
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: Some("/unexpected"),
            connect_src: None,
        },
        ("GET" | "HEAD", "/unexpected") => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: COMPLETE_PAGE.as_bytes(),
            include_body: request.method == "GET",
            allow: None,
            location: None,
            connect_src: None,
        },
        ("GET" | "HEAD", "/authenticated") => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: AUTHENTICATED_PAGE.as_bytes(),
            include_body: request.method == "GET",
            allow: None,
            location: None,
            connect_src: None,
        },
        ("GET" | "HEAD", "/mutation") if fixtures.is_active(now) => Response {
            status: 303,
            reason: "See Other",
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: Some("/mutation/created"),
            connect_src: None,
        },
        ("GET" | "HEAD", "/mutation") => mutation_empty_response(request.method),
        ("GET" | "HEAD", "/mutation/network-guard") if fixtures.is_active(now) => Response {
            status: 303,
            reason: "See Other",
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: Some("/mutation/created"),
            connect_src: None,
        },
        ("GET" | "HEAD", "/mutation/network-guard") => {
            mutation_network_guard_response(request.method)
        }
        ("GET" | "HEAD", "/mutation/created") if fixtures.is_active(now) => {
            mutation_created_response(request.method)
        }
        ("GET" | "HEAD", "/mutation/created") => Response {
            status: 303,
            reason: "See Other",
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: Some("/mutation/empty"),
            connect_src: None,
        },
        ("GET" | "HEAD", "/mutation/empty") if fixtures.is_active(now) => Response {
            status: 303,
            reason: "See Other",
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: Some("/mutation/created"),
            connect_src: None,
        },
        ("GET" | "HEAD", "/mutation/empty") => mutation_empty_response(request.method),
        ("POST", "/mutation/create") => {
            if !is_form_content_type(request.content_type) {
                return Response::empty(415, "Unsupported Media Type");
            }
            let fixture_name = match decode_fixture_name(request.body) {
                Ok(name) => name,
                Err(()) => return Response::empty(422, "Unprocessable Content"),
            };
            if fixtures.create(fixture_name, now).is_err() {
                return Response::empty(409, "Conflict");
            }
            Response {
                status: 303,
                reason: "See Other",
                media_type: "text/plain; charset=utf-8",
                body: b"",
                include_body: false,
                allow: None,
                location: Some("/mutation/created"),
                connect_src: None,
            }
        }
        ("POST", "/mutation/delete") => {
            if !is_form_content_type(request.content_type) || !request.body.is_empty() {
                return Response::empty(415, "Unsupported Media Type");
            }
            fixtures.clear();
            Response {
                status: 303,
                reason: "See Other",
                media_type: "text/plain; charset=utf-8",
                body: b"",
                include_body: false,
                allow: None,
                location: Some("/mutation/empty"),
                connect_src: None,
            }
        }
        ("GET" | "HEAD", "/healthz") => Response {
            status: 200,
            reason: "OK",
            media_type: "text/plain; charset=utf-8",
            body: b"ready\n",
            include_body: request.method == "GET",
            allow: None,
            location: None,
            connect_src: None,
        },
        ("GET" | "HEAD", "/favicon.ico") => Response::empty(204, "No Content"),
        (method, path)
            if matches!(
                path,
                "/" | "/complete"
                    | "/redirect"
                    | "/unexpected"
                    | "/authenticated"
                    | "/mutation"
                    | "/mutation/network-guard"
                    | "/mutation/empty"
                    | "/mutation/created"
                    | "/healthz"
                    | "/favicon.ico"
            ) && !matches!(method, "GET" | "HEAD") =>
        {
            method_not_allowed("GET, HEAD")
        }
        (method, "/mutation/create" | "/mutation/delete") if method != "POST" => {
            method_not_allowed("POST")
        }
        _ => Response::empty(404, "Not Found"),
    }
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut response = Response::empty(405, "Method Not Allowed");
    response.allow = Some(allow);
    response
}

fn mutation_empty_response(method: &str) -> Response {
    Response {
        status: 200,
        reason: "OK",
        media_type: "text/html; charset=utf-8",
        body: MUTATION_EMPTY_PAGE.as_bytes(),
        include_body: method == "GET",
        allow: None,
        location: None,
        connect_src: None,
    }
}

fn mutation_network_guard_response(method: &str) -> Response {
    let mut response = mutation_empty_response(method);
    response.connect_src = Some("http://127.0.0.1:* ws://127.0.0.1:*");
    response
}

fn mutation_created_response(method: &str) -> Response {
    Response {
        status: 200,
        reason: "OK",
        media_type: "text/html; charset=utf-8",
        body: MUTATION_CREATED_PAGE.as_bytes(),
        include_body: method == "GET",
        allow: None,
        location: None,
        connect_src: None,
    }
}

fn write_response(stream: &mut TcpStream, response: Response) -> Result<(), String> {
    let allow = response
        .allow
        .map_or_else(String::new, |value| format!("Allow: {value}\r\n"));
    let location = response
        .location
        .map_or_else(String::new, |value| format!("Location: {value}\r\n"));
    let connect_src = response
        .connect_src
        .map_or_else(String::new, |value| format!("connect-src {value}; "));
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}{}Cache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-crawlson-demo'; {}base-uri 'none'; form-action 'self'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.media_type,
        response.body.len(),
        allow,
        location,
        connect_src,
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|error| error.to_string())?;
    if response.include_body {
        stream
            .write_all(response.body)
            .map_err(|error| error.to_string())?;
    }
    stream.flush().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_store() -> FixtureStore {
        FixtureStore::new(FIXTURE_TTL)
    }

    fn form_post(path: &str, body: &str) -> Vec<u8> {
        format!(
            "POST {path} HTTP/1.1\r\nHost: example\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn rejects_non_loopback_bind_addresses() {
        assert!(validate_bind(IpAddr::V4(Ipv4Addr::LOCALHOST)).is_ok());
        assert!(validate_bind("::1".parse().unwrap()).is_ok());
        assert!(validate_bind("0.0.0.0".parse().unwrap()).is_err());
        assert!(validate_bind("192.0.2.10".parse().unwrap()).is_err());
    }

    #[test]
    fn preserves_read_only_demo_routes() {
        let mut fixtures = fixture_store();
        let page = route(b"GET / HTTP/1.1\r\nHost: example\r\n\r\n", &mut fixtures);
        assert_eq!(page.status, 200);
        assert!(page.include_body);
        assert!(
            std::str::from_utf8(page.body)
                .unwrap()
                .contains("id=\"action-button\"")
        );
        assert!(
            std::str::from_utf8(page.body)
                .unwrap()
                .contains("href=\"/complete\">Continue</a>")
        );
        assert!(
            std::str::from_utf8(page.body)
                .unwrap()
                .contains("id=\"redirect-button\" href=\"/redirect\"")
        );

        let complete = route(
            b"GET /complete?source=demo HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
        );
        assert_eq!(complete.status, 200);
        assert!(complete.include_body);
        let complete_body = std::str::from_utf8(complete.body).unwrap();
        assert!(complete_body.contains("id=\"completion-heading\">Journey complete</h1>"));
        assert!(complete_body.contains(
            "id=\"completion-status\">The same-origin Continue link reached its deterministic destination.</p>"
        ));

        let redirect = route(
            b"GET /redirect HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
        );
        assert_eq!(redirect.status, 302);
        assert_eq!(redirect.location, Some("/unexpected"));
        assert_eq!(
            route(b"GET /unexpected HTTP/1.1\r\n\r\n", &mut fixtures).status,
            200
        );

        let authenticated = route(
            b"GET /authenticated HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
        );
        assert_eq!(authenticated.status, 200);
        let authenticated_body = std::str::from_utf8(authenticated.body).unwrap();
        assert!(authenticated_body.contains("id=\"authenticated-role\">Sign in required</p>"));
        assert!(authenticated_body.contains("window.localStorage.getItem"));
        assert!(authenticated_body.contains("textContent = \"Viewer access\""));
        let authenticated_head = route(b"HEAD /authenticated HTTP/1.1\r\n\r\n", &mut fixtures);
        assert_eq!(authenticated_head.status, 200);
        assert!(!authenticated_head.include_body);

        let head = route(b"HEAD /healthz HTTP/1.1\r\n\r\n", &mut fixtures);
        assert_eq!(head.status, 200);
        assert!(!head.include_body);

        let post = route(b"POST / HTTP/1.1\r\n\r\n", &mut fixtures);
        assert_eq!(post.status, 405);
        assert_eq!(post.allow, Some("GET, HEAD"));

        assert_eq!(
            route(b"GET /missing HTTP/1.1\r\n\r\n", &mut fixtures).status,
            404
        );
    }

    #[test]
    fn creates_and_idempotently_deletes_a_disposable_fixture() {
        let mut fixtures = fixture_store();
        let started = Instant::now();

        let empty = route_at(
            b"GET /mutation HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
            started,
        );
        assert_eq!(empty.status, 200);
        let empty_body = std::str::from_utf8(empty.body).unwrap();
        assert!(empty_body.contains("id=\"fixture-empty\">Disposable fixture absent.</p>"));
        assert!(
            empty_body
                .contains("<form id=\"fixture-form\" action=\"/mutation/create\" method=\"post\">")
        );
        assert!(
            empty_body.contains("<input id=\"fixture-name\" name=\"fixture_name\" type=\"text\"")
        );
        assert!(empty_body.contains("<button id=\"create-fixture-button\" type=\"submit\">"));
        assert_eq!(empty.connect_src, None);

        let probe = route_at(
            b"GET /mutation/network-guard HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
            started,
        );
        assert_eq!(probe.status, 200);
        assert_eq!(
            probe.connect_src,
            Some("http://127.0.0.1:* ws://127.0.0.1:*")
        );
        let probe_body = std::str::from_utf8(probe.body).unwrap();
        assert!(probe_body.contains("void fetch(endpoint(\"fetch\")"));
        assert!(probe_body.contains("navigator.sendBeacon(endpoint(\"beacon\")"));
        assert!(probe_body.contains("new WebSocket(endpoint(\"websocket\")"));

        let create = route_at(
            &form_post("/mutation/create", "fixture_name=public-fixture_42"),
            &mut fixtures,
            started,
        );
        assert_eq!(create.status, 303);
        assert_eq!(create.location, Some("/mutation/created"));
        assert_eq!(
            fixtures
                .fixture
                .as_ref()
                .map(|fixture| fixture._name.as_str()),
            Some("public-fixture_42")
        );

        let created = route_at(
            b"GET /mutation/created HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
            started,
        );
        assert_eq!(created.status, 200);
        let created_body = std::str::from_utf8(created.body).unwrap();
        assert!(created_body.contains("id=\"fixture-result\">Disposable fixture created.</p>"));
        assert!(
            created_body
                .contains("<form id=\"cleanup-form\" action=\"/mutation/delete\" method=\"post\">")
        );
        assert!(created_body.contains("<button id=\"remove-fixture-button\" type=\"submit\">"));

        let duplicate = route_at(
            &form_post("/mutation/create", "fixture_name=second"),
            &mut fixtures,
            started,
        );
        assert_eq!(duplicate.status, 409);

        let delete = route_at(&form_post("/mutation/delete", ""), &mut fixtures, started);
        assert_eq!(delete.status, 303);
        assert_eq!(delete.location, Some("/mutation/empty"));
        assert!(fixtures.fixture.is_none());

        let repeated_delete = route_at(&form_post("/mutation/delete", ""), &mut fixtures, started);
        assert_eq!(repeated_delete.status, 303);
        assert_eq!(repeated_delete.location, Some("/mutation/empty"));
        assert!(fixtures.fixture.is_none());
    }

    #[test]
    fn expires_fixture_state_at_the_hard_deadline() {
        let mut fixtures = FixtureStore::new(Duration::from_secs(10));
        let started = Instant::now();
        assert_eq!(
            route_at(
                &form_post("/mutation/create", "fixture_name=expires"),
                &mut fixtures,
                started,
            )
            .status,
            303
        );
        assert!(fixtures.is_active(started + Duration::from_secs(9)));

        let expired = route_at(
            b"GET /mutation/created HTTP/1.1\r\nHost: example\r\n\r\n",
            &mut fixtures,
            started + Duration::from_secs(10),
        );
        assert_eq!(expired.status, 303);
        assert_eq!(expired.location, Some("/mutation/empty"));
        assert!(fixtures.fixture.is_none());
    }

    #[test]
    fn validates_bounded_url_encoded_public_fixture_tokens() {
        let started = Instant::now();
        let mut fixtures = fixture_store();
        let encoded = route_at(
            &form_post("/mutation/create", "fixture_name=public%2Dfixture%5F42"),
            &mut fixtures,
            started,
        );
        assert_eq!(encoded.status, 303);
        assert_eq!(
            fixtures
                .fixture
                .as_ref()
                .map(|fixture| fixture._name.as_str()),
            Some("public-fixture_42")
        );

        for invalid_body in [
            "fixture_name=",
            "fixture_name=has+space",
            "fixture_name=has%2Fslash",
            "fixture_name=%GG",
            "fixture_name=one&fixture_name=two",
            "unexpected=value",
        ] {
            let mut fixtures = fixture_store();
            let response = route_at(
                &form_post("/mutation/create", invalid_body),
                &mut fixtures,
                started,
            );
            assert_eq!(response.status, 422, "accepted {invalid_body}");
            assert!(fixtures.fixture.is_none());
        }

        let mut fixtures = fixture_store();
        let oversized_name = format!("fixture_name={}", "a".repeat(65));
        assert_eq!(
            route_at(
                &form_post("/mutation/create", &oversized_name),
                &mut fixtures,
                started,
            )
            .status,
            422
        );

        let wrong_media_type = b"POST /mutation/create HTTP/1.1\r\nContent-Type: text/plain\r\nContent-Length: 19\r\n\r\nfixture_name=public";
        assert_eq!(
            route_at(wrong_media_type, &mut fixtures, started).status,
            415
        );

        let oversized_body = "a".repeat(MAX_FORM_BODY_BYTES + 1);
        assert_eq!(
            route_at(
                &form_post("/mutation/create", &oversized_body),
                &mut fixtures,
                started,
            )
            .status,
            413
        );
    }

    #[test]
    fn normalizes_nonblocking_request_streams_before_reading() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            thread::sleep(Duration::from_millis(50));
            stream
                .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });

        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut fixtures = fixture_store();
        serve(stream, &mut fixtures).unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("ready\n"));
    }

    #[test]
    fn reads_a_bounded_post_body_and_keeps_forms_same_origin() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let request = form_post("/mutation/create", "fixture_name=over-the-wire");
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(&request).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });

        let (stream, _) = listener.accept().unwrap();
        let mut fixtures = fixture_store();
        serve(stream, &mut fixtures).unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 303 See Other\r\n"));
        assert!(response.contains("Location: /mutation/created\r\n"));
        assert!(response.contains("form-action 'self'"));
        assert_eq!(
            fixtures
                .fixture
                .as_ref()
                .map(|fixture| fixture._name.as_str()),
            Some("over-the-wire")
        );
    }
}
