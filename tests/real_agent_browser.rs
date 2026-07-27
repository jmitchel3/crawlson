use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;

const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Crawlson local fixture</title></head>
<body><main><h1 id="heading">Hello</h1><button id="action">Continue</button></main></body>
</html>"#;

#[test]
#[ignore = "requires a local agent-browser 0.26.x installation and browser runtime"]
fn real_agent_browser_runs_the_authorized_local_fixture() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let server = thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => respond(stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("local fixture accept failed: {error}"),
            }
        }
    });

    let directory = tempfile::tempdir().unwrap();
    let origin = format!("http://{address}");
    let journey = directory.path().join("journey.toml");
    std::fs::write(
        &journey,
        format!(
            r##"schema_version = 1

[journey]
id = "fixture.real-browser"
revision = 1
title = "Real browser fixture"
purpose = "Prove the agent-browser process boundary against visible local UI."
expected_outcome = "The heading and action are visible."
mode = "read_only"

[target]
origin = "{origin}"

[evidence]
trace = true
diagnostics = true

[[steps]]
id = "open"
title = "Open fixture"
action = {{ type = "navigate", path = "/" }}

[[steps]]
id = "heading"
title = "Check visible heading"
action = {{ type = "check_text", selector = "#heading", expected = "Hello", comparison = "exact" }}

[[steps]]
id = "action"
title = "Capture visible action"
guide_instruction = "Review the highlighted action."
action = {{ type = "capture", selector = "#action", alt_text = "Continue button highlighted in red" }}
"##
        ),
    )
    .unwrap();

    let mut command = Command::new(cargo_bin("crawlson"));
    command
        .args(["--json", "run"])
        .arg(&journey)
        .arg("--allow-origin")
        .arg(&origin)
        .arg("--output-dir")
        .arg(directory.path().join("runs"));
    if let Some(executable) = std::env::var_os("AGENT_BROWSER_REAL_BIN") {
        command.arg("--agent-browser").arg(executable);
    }
    let output = command.output().unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "passed");
    assert!(
        report["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|artifact| artifact["kind"] == "focused_screenshot")
    );
}

fn respond(mut stream: TcpStream) {
    let mut request = [0_u8; 4096];
    let _ = stream.read(&mut request);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        PAGE.len(),
        PAGE
    );
    stream.write_all(response.as_bytes()).unwrap();
}
