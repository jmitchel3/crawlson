use std::path::PathBuf;
use std::process::Command;
use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn canonical_and_alias_versions_match_the_package() {
    let crawlson = Command::new(cargo_bin("crawlson"))
        .arg("--version")
        .output()
        .unwrap();
    let clson = Command::new(cargo_bin("clson"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(crawlson.status.success());
    assert!(clson.status.success());
    assert_eq!(crawlson.stdout, clson.stdout);
    assert_eq!(
        String::from_utf8(crawlson.stdout).unwrap(),
        format!("crawlson {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn canonical_and_alias_json_version_reports_match() {
    let crawlson = command_output("crawlson", &["--json", "version"]);
    let clson = command_output("clson", &["--json", "version"]);
    assert!(crawlson.status.success());
    assert!(clson.status.success());
    assert_eq!(crawlson.stdout, clson.stdout);

    let report: Value = serde_json::from_slice(&crawlson.stdout).unwrap();
    assert_eq!(report["name"], "crawlson");
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["schema_version"], 1);
}

#[test]
fn invalid_arguments_use_the_stable_usage_exit() {
    let output = command_output("crawlson", &["does-not-exist"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        predicate::str::contains("unrecognized subcommand")
            .eval(&String::from_utf8_lossy(&output.stderr))
    );
}

#[test]
fn doctor_accepts_the_supported_agent_browser_range() {
    let fixture = FakeAgentBrowser::compile();
    let output = Command::new(cargo_bin("crawlson"))
        .args(["--json", "doctor", "--agent-browser"])
        .arg(&fixture.binary)
        .env("FAKE_AGENT_BROWSER_OUTPUT", "agent-browser 0.26.9")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "pass");
    assert_eq!(report["checks"][0]["detected_version"], "0.26.9");
}

#[test]
fn doctor_rejects_unsupported_or_malformed_versions_without_dirtying_json() {
    let fixture = FakeAgentBrowser::compile();
    for value in [
        "agent-browser 0.27.0",
        "agent-browser 0.26.0-alpha.1",
        "0.26.0",
    ] {
        let output = Command::new(cargo_bin("clson"))
            .args(["--json", "doctor", "--agent-browser"])
            .arg(&fixture.binary)
            .env("FAKE_AGENT_BROWSER_OUTPUT", value)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1), "{value}");
        assert!(output.stderr.is_empty(), "{value}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["status"], "fail", "{value}");
    }
}

#[test]
fn doctor_missing_executable_is_an_explicit_failure() {
    let output = command_output(
        "crawlson",
        &[
            "--json",
            "doctor",
            "--agent-browser",
            "path-that-does-not-exist/agent-browser",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "fail");
}

#[test]
fn doctor_rejects_nonzero_and_oversized_version_probes() {
    let fixture = FakeAgentBrowser::compile();
    let nonzero = Command::new(cargo_bin("crawlson"))
        .args(["--json", "doctor", "--agent-browser"])
        .arg(&fixture.binary)
        .env("FAKE_AGENT_BROWSER_EXIT", "7")
        .output()
        .unwrap();
    assert_eq!(nonzero.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&nonzero.stdout).unwrap()["status"],
        "fail"
    );

    let oversized = Command::new(cargo_bin("crawlson"))
        .args(["--json", "doctor", "--agent-browser"])
        .arg(&fixture.binary)
        .env(
            "FAKE_AGENT_BROWSER_OUTPUT",
            format!("agent-browser 0.26.0{}", " ".repeat(4096)),
        )
        .output()
        .unwrap();
    assert_eq!(oversized.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&oversized.stdout).unwrap()["status"],
        "fail"
    );
}

#[test]
fn explicit_offline_upgrade_makes_no_network_request_and_keeps_json_clean() {
    for name in ["crawlson", "clson"] {
        let output = command_output(name, &["--json", "upgrade", "--offline"]);
        assert_eq!(output.status.code(), Some(1), "{name}");
        assert!(output.stderr.is_empty(), "{name}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["status"], "blocked", "{name}");
        assert!(
            report["message"].as_str().unwrap().contains("offline"),
            "{name}"
        );
    }
}

#[test]
fn run_requires_explicit_origin_authorization_before_driver_launch() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let output = run_command("crawlson", &journey, directory.path(), &fixture, &[]);

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "blocked");
    assert_eq!(report["reason"]["code"], "target_authorization_missing");
    assert!(!fixture.call_log().exists());
}

#[test]
fn authenticated_journey_is_explicitly_blocked_before_driver_launch() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", true);
    let output = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );

    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reason"]["code"], "authentication_unavailable");
    assert!(!fixture.call_log().exists());
}

#[test]
fn fake_process_runner_reports_pass_failure_and_redirect_honestly() {
    for (scenario, expected_text, exit, outcome, code) in [
        ("pass", "Hello", 0, "passed", "journey_passed"),
        ("fail_text", "Hello", 1, "failed", "checkpoint_failed"),
        ("hidden", "Hello", 1, "failed", "checkpoint_failed"),
        ("redirect", "Hello", 3, "blocked", "origin_not_authorized"),
        (
            "escaped_open_response",
            "Hello",
            3,
            "blocked",
            "origin_not_authorized",
        ),
        (
            "escaped_text_origin",
            "Hello",
            3,
            "blocked",
            "origin_not_authorized",
        ),
        (
            "escaped_visible_origin",
            "Hello",
            3,
            "blocked",
            "origin_not_authorized",
        ),
        (
            "domain_block",
            "Hello",
            3,
            "blocked",
            "origin_not_authorized",
        ),
    ] {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), expected_text, false);
        let output = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );

        assert_eq!(output.status.code(), Some(exit), "{scenario}");
        assert!(output.stderr.is_empty(), "{scenario}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["outcome"], outcome, "{scenario}");
        assert_eq!(report["reason"]["code"], code, "{scenario}");
        assert_eq!(report["cleanup"]["status"], "passed", "{scenario}");
        assert!(
            report["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|artifact| artifact["kind"] == "trace"),
            "{scenario}"
        );
        if outcome == "blocked" {
            let calls = fs::read_to_string(fixture.call_log()).unwrap();
            assert!(
                !calls
                    .lines()
                    .any(|line| line.ends_with("\tconsole") || line.ends_with("\terrors")),
                "diagnostics ran after safety block in {scenario}"
            );
        }
    }
}

#[test]
fn fake_process_runner_maps_protocol_limits_timeout_and_cleanup_to_error() {
    for (scenario, extra, code) in [
        ("malformed", vec![], "driver_protocol"),
        ("oversized", vec![], "driver_output_limit"),
        ("oversized_stderr", vec![], "driver_output_limit"),
        ("command_error", vec![], "driver_command_failed"),
        ("success_with_error", vec![], "driver_protocol"),
        ("failure_with_data", vec![], "driver_protocol"),
        ("failure_without_error", vec![], "driver_protocol"),
        ("exit_success_envelope_failure", vec![], "driver_protocol"),
        ("exit_failure_envelope_success", vec![], "driver_protocol"),
        ("confirmation_required", vec![], "driver_protocol"),
        ("success_missing_data", vec![], "driver_protocol"),
        ("invalid_box", vec![], "driver_protocol"),
        (
            "timeout",
            vec!["--action-timeout-seconds", "1"],
            "driver_timeout",
        ),
        (
            "prepare_timeout",
            vec!["--action-timeout-seconds", "1"],
            "driver_timeout",
        ),
        ("prepare_error", vec![], "driver_command_failed"),
        ("cleanup_fail", vec![], "cleanup_failed"),
    ] {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let mut arguments = vec!["--allow-origin", "http://127.0.0.1:4173"];
        arguments.extend(extra);
        let output = run_command("crawlson", &journey, directory.path(), &fixture, &arguments);

        assert_eq!(output.status.code(), Some(4), "{scenario}");
        assert!(output.stderr.is_empty(), "{scenario}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["outcome"], "error", "{scenario}");
        assert_eq!(report["reason"]["code"], code, "{scenario}");
        if scenario == "cleanup_fail" {
            assert_eq!(report["execution_outcome"], "passed");
            assert_eq!(report["cleanup"]["status"], "failed");
        } else {
            assert_eq!(report["cleanup"]["status"], "passed", "{scenario}");
        }
        if matches!(scenario, "prepare_timeout" | "prepare_error") {
            let calls = fs::read_to_string(fixture.call_log()).unwrap();
            assert!(
                !calls
                    .lines()
                    .any(|line| line.ends_with("\tconsole") || line.ends_with("\terrors")),
                "diagnostics ran after prepare failure in {scenario}"
            );
        }
    }
}

#[test]
fn clson_run_preserves_outcome_and_exit_contract() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let output = run_command(
        "clson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "passed");
    assert_eq!(report["crawlson_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn required_diagnostics_and_trace_fail_closed() {
    for (scenario, code) in [
        ("diagnostics_error", "diagnostics_failed"),
        ("page_errors_error", "diagnostics_failed"),
        ("trace_stop_error", "trace_finalization_failed"),
        ("trace_zero", "trace_finalization_failed"),
        ("trace_mismatch", "trace_finalization_failed"),
        ("trace_malformed", "trace_finalization_failed"),
        ("trace_empty", "trace_finalization_failed"),
        ("trace_path_escape", "trace_finalization_failed"),
    ] {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let output = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        assert_eq!(output.status.code(), Some(4), "{scenario}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["outcome"], "error", "{scenario}");
        assert_eq!(report["execution_outcome"], "passed", "{scenario}");
        assert_eq!(report["reason"]["code"], code, "{scenario}");
        assert_eq!(report["cleanup"]["status"], "passed", "{scenario}");
    }
}

#[test]
fn focused_capture_preserves_raw_png_and_reproducible_overlay_metadata() {
    let fixture = FakeAgentBrowser::compile();
    fixture.enable_screenshot();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let mut source = fs::read_to_string(&journey).unwrap();
    source.push_str(
        r##"

[[steps]]
id = "capture-action"
title = "Capture the action area"
guide_instruction = "Use the highlighted control."
action = { type = "capture", selector = "#action", alt_text = "Highlighted action area" }
"##,
    );
    fs::write(&journey, source).unwrap();

    let output = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let artifacts = report["artifacts"].as_array().unwrap();
    let raw = artifacts
        .iter()
        .find(|artifact| artifact["kind"] == "raw_screenshot")
        .unwrap();
    let focused = artifacts
        .iter()
        .find(|artifact| artifact["kind"] == "focused_screenshot")
        .unwrap();
    let metadata = artifacts
        .iter()
        .find(|artifact| artifact["kind"] == "focus_metadata")
        .unwrap();

    let raw_bytes = fs::read(root.join(raw["path"].as_str().unwrap())).unwrap();
    let focused_bytes = fs::read(root.join(focused["path"].as_str().unwrap())).unwrap();
    assert_ne!(raw_bytes, focused_bytes);
    let overlay: Value =
        serde_json::from_slice(&fs::read(root.join(metadata["path"].as_str().unwrap())).unwrap())
            .unwrap();
    assert_eq!(overlay["renderer_algorithm"], "focus-overlay-v1");
    assert_eq!(overlay["mask_rgba"], serde_json::json!([0, 0, 0, 166]));
    assert_eq!(
        overlay["outline_rgba"],
        serde_json::json!([255, 45, 45, 255])
    );
    assert_eq!(overlay["source"]["sha256"], raw["sha256"]);
    assert_eq!(focused["source_artifact"], raw["path"]);
}

#[test]
fn published_report_schema_accepts_every_terminal_outcome_and_preflight_report() {
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/run-report-v1.schema.json")).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    for (scenario, authorization) in [
        ("pass", true),
        ("fail_text", true),
        ("domain_block", true),
        ("malformed", true),
        ("invalid_box", true),
        ("cleanup_fail", true),
        ("pass", false),
    ] {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let arguments = if authorization {
            vec!["--allow-origin", "http://127.0.0.1:4173"]
        } else {
            Vec::new()
        };
        let output = run_command("crawlson", &journey, directory.path(), &fixture, &arguments);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Err(error) = validator.validate(&report) {
            panic!("{scenario} report did not match published schema: {error}");
        }
    }

    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let invalid = fs::read_to_string(&journey)
        .unwrap()
        .replace("fixture.read-home", "INVALID ID");
    fs::write(&journey, invalid).unwrap();
    let output = run_command("crawlson", &journey, directory.path(), &fixture, &[]);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reason"]["code"], "journey_invalid");
    assert!(report["journey"].get("id").is_none());
    validator.validate(&report).unwrap();
}

#[test]
fn unauthorized_observed_urls_do_not_disclose_credentials_or_paths() {
    let fixture = FakeAgentBrowser::compile();
    fixture.set_scenario("credential_current_url");
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let output = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );

    assert_eq!(output.status.code(), Some(3));
    let serialized = String::from_utf8(output.stdout).unwrap();
    for secret in ["user", "secret", "private/path", "token", "hidden"] {
        assert!(!serialized.contains(secret), "report leaked {secret}");
    }
    let report: Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        report["steps"][0]["observation"]["observed_url"],
        "unauthorized-origin"
    );
}

fn command_output(name: &str, arguments: &[&str]) -> std::process::Output {
    Command::new(cargo_bin(name))
        .args(arguments)
        .output()
        .unwrap()
}

fn run_command(
    name: &str,
    journey: &Path,
    directory: &Path,
    fixture: &FakeAgentBrowser,
    extra: &[&str],
) -> std::process::Output {
    let mut command = Command::new(cargo_bin(name));
    command
        .args(["--json", "run"])
        .arg(journey)
        .arg("--output-dir")
        .arg(directory.join("runs"))
        .arg("--agent-browser")
        .arg(&fixture.binary)
        .args(extra)
        .env("CRAWLSON_UNSAFE_TEST_ENV", "must-be-scrubbed")
        .output()
        .unwrap()
}

fn write_journey(directory: &Path, expected: &str, authenticated: bool) -> PathBuf {
    let authentication = if authenticated {
        "\n[authentication]\nprovider = \"fixture\"\nrole = \"viewer\"\n"
    } else {
        ""
    };
    let source = format!(
        r#"
schema_version = 1

[journey]
id = "fixture.read-home"
revision = 1
title = "Read home"
purpose = "Verify the fixture through its visible UI."
expected_outcome = "The heading is visible."
mode = "read_only"

[target]
origin = "http://127.0.0.1:4173"
{authentication}
[evidence]
trace = true
diagnostics = true

[[steps]]
id = "open"
title = "Open the fixture"
action = {{ type = "navigate", path = "/" }}

[[steps]]
id = "heading"
title = "Check the heading"
action = {{ type = "check_text", selector = "h1", expected = "{expected}", comparison = "exact" }}

[[steps]]
id = "capture-heading"
title = "Capture the heading"
guide_instruction = "Review the highlighted heading."
action = {{ type = "capture", selector = "h1", alt_text = "Highlighted heading" }}
"#
    );
    let path = directory.join("journey.toml");
    fs::write(&path, source).unwrap();
    path
}

struct FakeAgentBrowser {
    _directory: TempDir,
    binary: PathBuf,
}

impl FakeAgentBrowser {
    fn compile() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut binary = directory.path().join("fake-agent-browser");
        if cfg!(windows) {
            binary.set_extension("exe");
        }
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_agent_browser.rs");
        let status = Command::new("rustc")
            .arg("--edition=2024")
            .arg(source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap();
        assert!(status.success());
        let fixture = Self {
            _directory: directory,
            binary,
        };
        fixture.enable_screenshot();
        fixture
    }

    fn set_scenario(&self, scenario: &str) {
        fs::write(self.binary.parent().unwrap().join("scenario"), scenario).unwrap();
    }

    fn call_log(&self) -> PathBuf {
        self.binary.parent().unwrap().join("calls.log")
    }

    fn enable_screenshot(&self) {
        let path = self.binary.parent().unwrap().join("screenshot.png");
        let file = fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, 1280, 720);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[200, 180, 160, 255].repeat(1280 * 720))
            .unwrap();
        writer.finish().unwrap();
    }
}
