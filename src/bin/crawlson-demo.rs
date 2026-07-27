use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use clap::Parser;
use serde::Serialize;

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

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

    while !stopping.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = serve(stream) {
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

fn serve(mut stream: TcpStream) -> Result<(), String> {
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
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let response = route(&request);
    write_response(&mut stream, response)
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
        }
    }
}

fn route(request: &[u8]) -> Response {
    let Ok(request) = std::str::from_utf8(request) else {
        return Response::empty(400, "Bad Request");
    };
    let Some(line) = request.lines().next() else {
        return Response::empty(400, "Bad Request");
    };
    let mut parts = line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Response::empty(400, "Bad Request");
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || !target.starts_with('/') {
        return Response::empty(400, "Bad Request");
    }
    if !matches!(method, "GET" | "HEAD") {
        let mut response = Response::empty(405, "Method Not Allowed");
        response.allow = Some("GET, HEAD");
        return response;
    }
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    match path {
        "/" => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: PAGE.as_bytes(),
            include_body: method == "GET",
            allow: None,
            location: None,
        },
        "/complete" => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: COMPLETE_PAGE.as_bytes(),
            include_body: method == "GET",
            allow: None,
            location: None,
        },
        "/redirect" => Response {
            status: 302,
            reason: "Found",
            media_type: "text/plain; charset=utf-8",
            body: b"",
            include_body: false,
            allow: None,
            location: Some("/unexpected"),
        },
        "/unexpected" => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: COMPLETE_PAGE.as_bytes(),
            include_body: method == "GET",
            allow: None,
            location: None,
        },
        "/authenticated" => Response {
            status: 200,
            reason: "OK",
            media_type: "text/html; charset=utf-8",
            body: AUTHENTICATED_PAGE.as_bytes(),
            include_body: method == "GET",
            allow: None,
            location: None,
        },
        "/healthz" => Response {
            status: 200,
            reason: "OK",
            media_type: "text/plain; charset=utf-8",
            body: b"ready\n",
            include_body: method == "GET",
            allow: None,
            location: None,
        },
        "/favicon.ico" => Response::empty(204, "No Content"),
        _ => Response::empty(404, "Not Found"),
    }
}

fn write_response(stream: &mut TcpStream, response: Response) -> Result<(), String> {
    let allow = response
        .allow
        .map_or_else(String::new, |value| format!("Allow: {value}\r\n"));
    let location = response
        .location
        .map_or_else(String::new, |value| format!("Location: {value}\r\n"));
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}{}Cache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-crawlson-demo'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.media_type,
        response.body.len(),
        allow,
        location,
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

    #[test]
    fn rejects_non_loopback_bind_addresses() {
        assert!(validate_bind(IpAddr::V4(Ipv4Addr::LOCALHOST)).is_ok());
        assert!(validate_bind("::1".parse().unwrap()).is_ok());
        assert!(validate_bind("0.0.0.0".parse().unwrap()).is_err());
        assert!(validate_bind("192.0.2.10".parse().unwrap()).is_err());
    }

    #[test]
    fn serves_only_read_only_demo_routes() {
        let page = route(b"GET / HTTP/1.1\r\nHost: example\r\n\r\n");
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

        let complete = route(b"GET /complete?source=demo HTTP/1.1\r\nHost: example\r\n\r\n");
        assert_eq!(complete.status, 200);
        assert!(complete.include_body);
        let complete_body = std::str::from_utf8(complete.body).unwrap();
        assert!(complete_body.contains("id=\"completion-heading\">Journey complete</h1>"));
        assert!(complete_body.contains(
            "id=\"completion-status\">The same-origin Continue link reached its deterministic destination.</p>"
        ));

        let redirect = route(b"GET /redirect HTTP/1.1\r\nHost: example\r\n\r\n");
        assert_eq!(redirect.status, 302);
        assert_eq!(redirect.location, Some("/unexpected"));
        assert_eq!(route(b"GET /unexpected HTTP/1.1\r\n\r\n").status, 200);

        let authenticated = route(b"GET /authenticated HTTP/1.1\r\nHost: example\r\n\r\n");
        assert_eq!(authenticated.status, 200);
        let authenticated_body = std::str::from_utf8(authenticated.body).unwrap();
        assert!(authenticated_body.contains("id=\"authenticated-role\">Sign in required</p>"));
        assert!(authenticated_body.contains("window.localStorage.getItem"));
        assert!(authenticated_body.contains("textContent = \"Viewer access\""));
        let authenticated_head = route(b"HEAD /authenticated HTTP/1.1\r\n\r\n");
        assert_eq!(authenticated_head.status, 200);
        assert!(!authenticated_head.include_body);

        let head = route(b"HEAD /healthz HTTP/1.1\r\n\r\n");
        assert_eq!(head.status, 200);
        assert!(!head.include_body);

        let post = route(b"POST / HTTP/1.1\r\n\r\n");
        assert_eq!(post.status, 405);
        assert_eq!(post.allow, Some("GET, HEAD"));

        assert_eq!(route(b"GET /missing HTTP/1.1\r\n\r\n").status, 404);
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
        serve(stream).unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("ready\n"));
    }
}
