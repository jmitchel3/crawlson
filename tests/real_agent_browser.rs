use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;
use png::{ColorType, Decoder, Transformations};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

const REAL_BROWSER_MODE: &str = "CRAWLSON_REAL_BROWSER";
const REAL_CLI_BIN: &str = "CRAWLSON_REAL_CLI_BIN";
const REAL_DEMO_BIN: &str = "CRAWLSON_REAL_DEMO_BIN";
const REQUIRED_MODE: &str = "required";
const SKIP_MODE: &str = "skip";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(180);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const DOCUMENTED_DEMO_ORIGIN: &str = "http://127.0.0.1:4173";

#[test]
#[ignore = "set CRAWLSON_REAL_BROWSER=required and run with --ignored; use =skip only for an explicit portable opt-out"]
fn real_agent_browser_runs_the_complete_loopback_demo() {
    if explicitly_skipped() {
        return;
    }

    let agent_browser = agent_browser_executable();
    require_supported_agent_browser(agent_browser.as_os_str());

    let demo = DemoProcess::start();
    let directory = tempfile::tempdir().expect("create real-browser test directory");
    let pass_journey = copy_demo_journey(directory.path(), "demo-pass.toml", &demo.origin);
    let fail_journey = copy_demo_journey(directory.path(), "demo-fail.toml", &demo.origin);

    let pass = run_journey(
        &pass_journey,
        &directory.path().join("pass-runs"),
        agent_browser.as_os_str(),
        Some(&demo.origin),
    );
    assert_exit(&pass, 0, "passing real-browser journey");
    let pass_report = json_stdout(&pass, "passing real-browser journey");
    assert_eq!(pass_report["outcome"], "passed");
    assert_eq!(pass_report["execution_outcome"], "passed");
    assert_eq!(pass_report["cleanup"]["status"], "passed");
    assert_eq!(pass_report["driver"]["name"], "agent-browser");
    assert_driver_ran(&pass_report);
    let pass_root = run_root(&pass_report);
    let pass_evidence = verify_evidence(&pass_root, &pass_report);
    verify_focus_pixels(&pass_evidence);

    let pass_render = render_journey(&pass_root, &pass_journey);
    assert_exit(&pass_render, 0, "passing render");
    let pass_render_report = json_stdout(&pass_render, "passing render");
    assert_eq!(pass_render_report["status"], "guide_ready");
    assert_eq!(pass_render_report["publishable"], true);
    assert_output_kinds(&pass_render_report, &["guide", "guide_image"]);
    let guide =
        fs::read_to_string(pass_root.join("render/guide.md")).expect("read real-browser guide");
    assert!(guide.contains("Review the highlighted Continue action\\."));
    assert!(guide.contains("![Continue action highlighted in red](001-focused.png)"));
    assert_eq!(
        fs::read(pass_root.join("render/001-focused.png")).expect("read rendered guide image"),
        fs::read(&pass_evidence.focused).expect("read focused evidence")
    );

    let failed = run_journey(
        &fail_journey,
        &directory.path().join("fail-runs"),
        agent_browser.as_os_str(),
        Some(&demo.origin),
    );
    assert_exit(&failed, 1, "failing real-browser journey");
    let failed_report = json_stdout(&failed, "failing real-browser journey");
    assert_eq!(failed_report["outcome"], "failed");
    assert_eq!(failed_report["execution_outcome"], "failed");
    assert_eq!(failed_report["reason"]["code"], "checkpoint_failed");
    assert_eq!(failed_report["cleanup"]["status"], "passed");
    assert_driver_ran(&failed_report);
    let failed_root = run_root(&failed_report);
    let failed_evidence = verify_evidence(&failed_root, &failed_report);
    verify_focus_pixels(&failed_evidence);

    let failed_render = render_journey(&failed_root, &fail_journey);
    assert_exit(&failed_render, 1, "failing render");
    let failed_render_report = json_stdout(&failed_render, "failing render");
    assert_eq!(failed_render_report["status"], "findings_ready");
    assert_eq!(failed_render_report["publishable"], true);
    assert_output_kinds(
        &failed_render_report,
        &["findings_json", "findings_markdown"],
    );
    verify_findings(&failed_root);

    let blocked = run_journey(
        &pass_journey,
        &directory.path().join("blocked-runs"),
        agent_browser.as_os_str(),
        None,
    );
    assert_exit(&blocked, 3, "blocked real-browser journey");
    let blocked_report = json_stdout(&blocked, "blocked real-browser journey");
    assert_eq!(blocked_report["outcome"], "blocked");
    assert_eq!(
        blocked_report["reason"]["code"],
        "target_authorization_missing"
    );
    assert_eq!(blocked_report["cleanup"]["status"], "not_needed");
    assert!(
        blocked_report["driver"]["commands"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "an unauthorized target must be blocked before browser launch"
    );
    assert!(
        blocked_report["artifacts"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "a preflight block must not invent browser evidence"
    );

    demo.shutdown();
}

fn explicitly_skipped() -> bool {
    match std::env::var(REAL_BROWSER_MODE).as_deref() {
        Ok(REQUIRED_MODE) => false,
        Ok(SKIP_MODE) => {
            eprintln!("real agent-browser integration explicitly skipped via {REAL_BROWSER_MODE}");
            true
        }
        Ok(value) => panic!(
            "unsupported {REAL_BROWSER_MODE}={value:?}; use {REQUIRED_MODE:?} to run or {SKIP_MODE:?} to opt out"
        ),
        Err(std::env::VarError::NotPresent) => panic!(
            "the ignored real-browser test was explicitly selected; set {REAL_BROWSER_MODE}={REQUIRED_MODE} to require it or {REAL_BROWSER_MODE}={SKIP_MODE} to opt out"
        ),
        Err(error) => panic!("invalid {REAL_BROWSER_MODE}: {error}"),
    }
}

fn agent_browser_executable() -> PathBuf {
    let requested = std::env::var_os("AGENT_BROWSER_REAL_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("agent-browser"));
    let candidate = if requested.components().count() > 1 {
        requested
    } else {
        let path = std::env::var_os("PATH").expect("PATH is required to find agent-browser");
        std::env::split_paths(&path)
            .flat_map(|directory| {
                let plain = directory.join(&requested);
                #[cfg(windows)]
                let candidates = [plain.clone(), plain.with_extension("exe")];
                #[cfg(not(windows))]
                let candidates = [plain];
                candidates
            })
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| panic!("agent-browser was not found on PATH"))
    };
    candidate
        .canonicalize()
        .unwrap_or_else(|error| panic!("resolve {}: {error}", candidate.display()))
}

fn require_supported_agent_browser(executable: &OsStr) {
    let mut command = Command::new(crawlson_executable());
    command
        .args(["--json", "doctor", "--agent-browser"])
        .arg(executable);
    let output = command_output(command, "agent-browser version check");
    assert_exit(&output, 0, "agent-browser version check");
    let report = json_stdout(&output, "agent-browser version check");
    let check = &report["checks"][0];
    assert_eq!(check["name"], "agent-browser");
    assert_eq!(check["status"], "pass");
    assert!(
        check["detected_version"]
            .as_str()
            .is_some_and(|version| version.starts_with("0.26.")),
        "doctor did not report a supported agent-browser 0.26.x: {report}"
    );
}

struct DemoProcess {
    child: Option<Child>,
    origin: String,
}

impl DemoProcess {
    fn start() -> Self {
        let mut child = Command::new(demo_executable())
            .args(["--port", "0", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start crawlson-demo");
        let stdout = child.stdout.take().expect("capture crawlson-demo stdout");
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout)
                .read_line(&mut line)
                .map(|count| (count, line));
            let _ = sender.send(result);
        });
        let readiness = match receiver.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok((count, line))) if count > 0 => line,
            Ok(Ok(_)) => startup_failure(&mut child, "demo exited before its readiness line"),
            Ok(Err(error)) => startup_failure(
                &mut child,
                &format!("could not read readiness line: {error}"),
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let diagnostics = stop_failed_demo(&mut child);
                panic!(
                    "crawlson-demo did not become ready within {} seconds: {}",
                    STARTUP_TIMEOUT.as_secs(),
                    diagnostics
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                startup_failure(&mut child, "readiness reader disconnected")
            }
        };
        reader.join().expect("join demo readiness reader");

        let ready: Value = serde_json::from_str(readiness.trim()).unwrap_or_else(|error| {
            startup_failure(
                &mut child,
                &format!("malformed readiness JSON ({error}): {readiness:?}"),
            )
        });
        assert_eq!(ready["schema_version"], 1, "unexpected readiness: {ready}");
        assert_eq!(ready["status"], "ready", "unexpected readiness: {ready}");
        assert_eq!(
            ready["pid"].as_u64(),
            Some(u64::from(child.id())),
            "readiness PID does not identify the spawned demo: {ready}"
        );
        let origin = ready["origin"]
            .as_str()
            .expect("demo readiness origin must be a string")
            .to_owned();
        let parsed = url::Url::parse(&origin).expect("demo readiness origin must be a URL");
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert!(parsed.port().is_some_and(|port| port != 0));
        assert_eq!(parsed.path(), "/");
        assert!(parsed.query().is_none() && parsed.fragment().is_none());
        assert!(
            child
                .try_wait()
                .expect("query crawlson-demo status")
                .is_none(),
            "crawlson-demo exited immediately after readiness"
        );

        Self {
            child: Some(child),
            origin,
        }
    }

    fn shutdown(mut self) {
        let mut child = self.child.take().expect("demo process is present");
        request_demo_shutdown(&mut child);
        let graceful = child
            .wait_timeout(SHUTDOWN_TIMEOUT)
            .expect("wait for crawlson-demo shutdown");
        if graceful.is_none() {
            force_kill(&mut child);
            let _ = child.wait_timeout(SHUTDOWN_TIMEOUT);
        }
        assert!(
            graceful.is_some_and(|status| status.success()),
            "crawlson-demo did not exit cleanly within {} seconds",
            SHUTDOWN_TIMEOUT.as_secs()
        );
    }
}

impl Drop for DemoProcess {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            request_demo_shutdown(child);
            if child
                .wait_timeout(SHUTDOWN_TIMEOUT)
                .ok()
                .flatten()
                .is_none()
            {
                force_kill(child);
                let _ = child.wait_timeout(SHUTDOWN_TIMEOUT);
            }
        }
    }
}

fn startup_failure(child: &mut Child, message: &str) -> ! {
    panic!("{message}: {}", stop_failed_demo(child));
}

fn stop_failed_demo(child: &mut Child) -> String {
    request_demo_shutdown(child);
    if child
        .wait_timeout(SHUTDOWN_TIMEOUT)
        .ok()
        .flatten()
        .is_none()
    {
        force_kill(child);
        let _ = child.wait_timeout(SHUTDOWN_TIMEOUT);
        return "demo did not stop cleanly; stderr was not read to keep teardown bounded"
            .to_owned();
    }
    child_stderr(child)
}

fn request_demo_shutdown(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        if status.is_err() || status.is_ok_and(|status| !status.success()) {
            force_kill(child);
        }
    }
    #[cfg(not(unix))]
    force_kill(child);
}

fn force_kill(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
}

fn child_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    stderr
}

fn copy_demo_journey(directory: &Path, name: &str, origin: &str) -> PathBuf {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name);
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
    assert_eq!(
        source.matches(DOCUMENTED_DEMO_ORIGIN).count(),
        1,
        "{} must contain the documented demo origin exactly once",
        source_path.display()
    );
    let journey = directory.join(name);
    fs::write(&journey, source.replace(DOCUMENTED_DEMO_ORIGIN, origin))
        .expect("write ephemeral demo journey");
    journey
}

fn run_journey(
    journey: &Path,
    output_directory: &Path,
    agent_browser: &OsStr,
    allowed_origin: Option<&str>,
) -> Output {
    let mut command = Command::new(crawlson_executable());
    command
        .args(["--json", "run"])
        .arg(journey)
        .arg("--output-dir")
        .arg(output_directory)
        .arg("--agent-browser")
        .arg(agent_browser);
    if let Some(origin) = allowed_origin {
        command.arg("--allow-origin").arg(origin);
    }
    command_output(command, "crawlson run")
}

fn render_journey(run_directory: &Path, journey: &Path) -> Output {
    let mut command = Command::new(crawlson_executable());
    command
        .args(["--json", "render"])
        .arg(run_directory)
        .arg("--journey")
        .arg(journey);
    command_output(command, "crawlson render")
}

fn crawlson_executable() -> PathBuf {
    selected_executable(REAL_CLI_BIN, "crawlson")
}

fn demo_executable() -> PathBuf {
    selected_executable(REAL_DEMO_BIN, "crawlson-demo")
}

fn selected_executable(variable: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(variable) {
        Some(path) => {
            let requested = PathBuf::from(path);
            assert!(
                requested.is_absolute(),
                "{variable} must name an absolute packaged executable"
            );
            requested
                .canonicalize()
                .unwrap_or_else(|error| panic!("resolve {variable}: {error}"))
        }
        None => cargo_bin(fallback),
    }
}

fn command_output(mut command: Command, label: &str) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("start {label}: {error}"));
    if child
        .wait_timeout(PROCESS_TIMEOUT)
        .unwrap_or_else(|error| panic!("wait for {label}: {error}"))
        .is_none()
    {
        force_kill(&mut child);
        let output = child
            .wait_with_output()
            .unwrap_or_else(|error| panic!("collect timed-out {label}: {error}"));
        panic!(
            "{label} exceeded {} seconds; stdout={} stderr={}",
            PROCESS_TIMEOUT.as_secs(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("collect {label} output: {error}"))
}

fn assert_exit(output: &Output, expected: i32, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{label}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json_stdout(output: &Output, label: &str) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{label} did not emit JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn run_root(report: &Value) -> PathBuf {
    let root = PathBuf::from(
        report["run_directory"]
            .as_str()
            .expect("run report must contain run_directory"),
    );
    assert!(
        root.is_dir(),
        "run directory does not exist: {}",
        root.display()
    );
    root
}

fn assert_driver_ran(report: &Value) {
    let commands = report["driver"]["commands"]
        .as_array()
        .expect("driver commands must be an array");
    assert!(!commands.is_empty());
    assert_eq!(commands[0]["capability"], "set_viewport");
    assert_eq!(commands[1]["capability"], "trace_start");
    assert_eq!(commands.last().unwrap()["capability"], "close");
    assert!(
        commands
            .iter()
            .all(|command| command["upstream_success"] == true)
    );
}

struct EvidencePaths {
    raw: PathBuf,
    focused: PathBuf,
    metadata: Value,
}

fn verify_evidence(root: &Path, report: &Value) -> EvidencePaths {
    let raw = verified_artifact(root, report, "raw_screenshot");
    let focused = verified_artifact(root, report, "focused_screenshot");
    let sidecar = verified_artifact(root, report, "focus_metadata");
    let trace = verified_artifact(root, report, "trace");
    let trace_document: Value = serde_json::from_slice(&fs::read(&trace).expect("read trace"))
        .expect("real agent-browser trace must be JSON");
    assert!(
        trace_document["traceEvents"]
            .as_array()
            .is_some_and(|events| !events.is_empty()),
        "real agent-browser trace must contain events"
    );

    let metadata: Value = serde_json::from_slice(&fs::read(&sidecar).expect("read focus sidecar"))
        .expect("focus sidecar must be JSON");
    assert_eq!(metadata["schema_version"], 1);
    assert_eq!(metadata["renderer_algorithm"], "focus-overlay-v1");
    assert_eq!(metadata["status"], "complete");
    assert_eq!(
        metadata["source"]["path"],
        artifact_relative(report, "raw_screenshot")
    );
    assert_eq!(
        metadata["derivative"]["path"],
        artifact_relative(report, "focused_screenshot")
    );
    assert_eq!(metadata["mask_rgba"], serde_json::json!([0, 0, 0, 166]));
    assert_eq!(
        metadata["outline_rgba"],
        serde_json::json!([255, 45, 45, 255])
    );

    EvidencePaths {
        raw,
        focused,
        metadata,
    }
}

fn verified_artifact(root: &Path, report: &Value, kind: &str) -> PathBuf {
    let artifact = report["artifacts"]
        .as_array()
        .expect("artifacts must be an array")
        .iter()
        .find(|artifact| artifact["kind"] == kind)
        .unwrap_or_else(|| panic!("run report omitted {kind}"));
    let path = root.join(
        artifact["path"]
            .as_str()
            .unwrap_or_else(|| panic!("{kind} path must be a string")),
    );
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read {kind}: {error}"));
    assert_eq!(
        artifact["size_bytes"].as_u64(),
        Some(bytes.len() as u64),
        "{kind} size mismatch"
    );
    assert_eq!(
        artifact["sha256"].as_str(),
        Some(hex_digest(&bytes).as_str()),
        "{kind} digest mismatch"
    );
    path
}

fn artifact_relative<'a>(report: &'a Value, kind: &str) -> &'a str {
    report["artifacts"]
        .as_array()
        .expect("artifacts must be an array")
        .iter()
        .find(|artifact| artifact["kind"] == kind)
        .and_then(|artifact| artifact["path"].as_str())
        .unwrap_or_else(|| panic!("run report omitted {kind}"))
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct DecodedPng {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn decode_png(path: &Path) -> DecodedPng {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut decoder = Decoder::new(BufReader::new(Cursor::new(bytes)));
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("decode PNG header");
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("determine PNG output size")
    ];
    let info = reader.next_frame(&mut buffer).expect("decode PNG frame");
    let pixels = u64::from(info.width) * u64::from(info.height);
    let frame = &buffer[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(usize::try_from(pixels * 4).expect("PNG is addressable"));
    match info.color_type {
        ColorType::Rgba => rgba.extend_from_slice(frame),
        ColorType::Rgb => {
            for value in frame.chunks_exact(3) {
                rgba.extend_from_slice(&[value[0], value[1], value[2], 255]);
            }
        }
        ColorType::Grayscale => {
            for value in frame {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            for value in frame.chunks_exact(2) {
                rgba.extend_from_slice(&[value[0], value[0], value[0], value[1]]);
            }
        }
        ColorType::Indexed => panic!("expanded PNG must not remain indexed"),
    }
    assert_eq!(rgba.len(), usize::try_from(pixels * 4).unwrap());
    DecodedPng {
        width: info.width,
        height: info.height,
        rgba,
    }
}

fn verify_focus_pixels(evidence: &EvidencePaths) {
    let raw = decode_png(&evidence.raw);
    let focused = decode_png(&evidence.focused);
    assert_eq!((focused.width, focused.height), (raw.width, raw.height));
    assert_eq!(
        evidence.metadata["image_width_px"].as_u64(),
        Some(u64::from(raw.width))
    );
    assert_eq!(
        evidence.metadata["image_height_px"].as_u64(),
        Some(u64::from(raw.height))
    );

    let target = &evidence.metadata["target_rect_px"];
    let focus = &evidence.metadata["focus_rect_px"];
    let left = u32_value(target, "left");
    let top = u32_value(target, "top");
    let right = u32_value(target, "right");
    let bottom = u32_value(target, "bottom");
    let stroke = evidence.metadata["outline_width_px"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .expect("outline width must be u32");
    assert!(right - left > stroke * 2 && bottom - top > stroke * 2);

    assert_eq!(pixel(&focused, left, top), [255, 45, 45, 255]);
    assert_eq!(
        pixel(&focused, left + stroke, top + stroke),
        pixel(&raw, left + stroke, top + stroke),
        "pixels inside the action area must remain unchanged"
    );

    let focus_left = u32_value(focus, "left");
    let focus_top = u32_value(focus, "top");
    let focus_right = u32_value(focus, "right");
    let focus_bottom = u32_value(focus, "bottom");
    let (outside_x, outside_y) = [
        (0, 0),
        (raw.width - 1, 0),
        (0, raw.height - 1),
        (raw.width - 1, raw.height - 1),
    ]
    .into_iter()
    .find(|(x, y)| *x < focus_left || *x >= focus_right || *y < focus_top || *y >= focus_bottom)
    .expect("focused action must leave visible surrounding pixels");
    let raw_outside = pixel(&raw, outside_x, outside_y);
    let focused_outside = pixel(&focused, outside_x, outside_y);
    for channel in 0..3 {
        assert_eq!(
            focused_outside[channel],
            ((u16::from(raw_outside[channel]) * 89 + 127) / 255) as u8,
            "surrounding pixel channel was not dimmed with the v1 mask"
        );
    }
    assert_eq!(focused_outside[3], raw_outside[3]);
    assert_ne!(focused_outside, raw_outside);
}

fn u32_value(object: &Value, name: &str) -> u32 {
    object[name]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_else(|| panic!("{name} must be u32"))
}

fn pixel(image: &DecodedPng, x: u32, y: u32) -> [u8; 4] {
    assert!(x < image.width && y < image.height);
    let offset = usize::try_from((u64::from(y) * u64::from(image.width) + u64::from(x)) * 4)
        .expect("pixel offset is addressable");
    image.rgba[offset..offset + 4]
        .try_into()
        .expect("pixel must contain four channels")
}

fn assert_output_kinds(report: &Value, expected: &[&str]) {
    let kinds: Vec<&str> = report["outputs"]
        .as_array()
        .expect("render outputs must be an array")
        .iter()
        .map(|output| {
            output["kind"]
                .as_str()
                .expect("output kind must be a string")
        })
        .collect();
    for kind in expected {
        assert!(
            kinds.contains(kind),
            "render output omitted {kind}: {report}"
        );
    }
}

fn verify_findings(root: &Path) {
    let findings: Value = serde_json::from_slice(
        &fs::read(root.join("render/findings.json")).expect("read real-browser findings JSON"),
    )
    .expect("findings output must be JSON");
    let finding = &findings["findings"][0];
    assert_eq!(finding["severity"], "untriaged");
    assert_eq!(finding["kind"], "text_mismatch");
    assert_eq!(finding["step"]["id"], "confirm-heading");
    let expected = finding["checkpoint"]["expected"]
        .as_str()
        .expect("failed checkpoint expected text must be a string");
    assert!(!expected.is_empty());
    assert_ne!(expected, "Welcome to the Crawlson demo");
    assert_eq!(finding["checkpoint"]["visible"], true);
    assert_eq!(finding["checkpoint"]["matched"], false);
    assert!(
        finding["checkpoint"]["observed_text_sha256"]
            .as_str()
            .is_some()
    );
    let reproduction = finding["reproduction_steps"]
        .as_array()
        .expect("finding reproduction must be an array");
    assert_eq!(reproduction.len(), 2);
    assert_eq!(reproduction[0]["action"]["type"], "navigate");
    assert_eq!(reproduction[1]["action"]["type"], "check_text");
    assert_eq!(reproduction[1]["action"]["selector"], "#welcome-heading");
    assert_eq!(reproduction[1]["action"]["expected"], expected);
    assert_eq!(reproduction[1]["status"], "failed");
    let evidence = finding["evidence"]
        .as_array()
        .expect("finding evidence must be an array");
    for kind in [
        "run_report",
        "trace",
        "raw_screenshot",
        "focused_screenshot",
        "focus_metadata",
    ] {
        assert!(
            evidence.iter().any(|item| item["kind"] == kind),
            "finding omitted {kind} evidence: {finding}"
        );
    }
    let markdown = fs::read_to_string(root.join("render/findings.md"))
        .expect("read real-browser findings Markdown");
    assert!(markdown.contains("Visible text did not match the declared checkpoint."));
}
