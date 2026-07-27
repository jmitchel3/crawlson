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
fn published_journey_v2_schema_accepts_the_example_and_rejects_ambiguous_extensions() {
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/journey-v2.schema.json")).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let document: toml::Value =
        toml::from_str(include_str!("../examples/read-only-journey.toml")).unwrap();
    let document = serde_json::to_value(document).unwrap();
    validator.validate(&document).unwrap();

    let mut non_capture_association = document.clone();
    non_capture_association["steps"][0]["evidence_for"] = serde_json::json!(["confirm-location"]);
    assert!(validator.validate(&non_capture_association).is_err());

    let mut query_path = document;
    query_path["steps"][0]["action"]["path"] = Value::String("/?secret=value".to_owned());
    assert!(validator.validate(&query_path).is_err());
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

#[test]
fn render_pass_is_deterministic_idempotent_and_alias_equivalent() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(run.status.success());
    let run_report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let original_root = PathBuf::from(run_report["run_directory"].as_str().unwrap());
    let root = directory.path().join("archived-run");
    fs::rename(&original_root, &root).unwrap();
    let calls_before = fs::read(fixture.call_log()).unwrap();

    let canonical = render_command("crawlson", &root, &journey);
    assert_eq!(
        canonical.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&canonical.stdout)
    );
    assert!(canonical.stderr.is_empty());
    let alias = render_command("clson", &root, &journey);
    assert_eq!(alias.status.code(), Some(0));
    assert_eq!(canonical.stdout, alias.stdout);
    assert_eq!(calls_before, fs::read(fixture.call_log()).unwrap());

    let result: Value = serde_json::from_slice(&canonical.stdout).unwrap();
    assert_eq!(result["status"], "guide_ready");
    assert_eq!(result["publishable"], true);
    assert_eq!(result["guide_steps"], 1);
    assert_eq!(
        result["report_sha256"],
        crawlson::journey::hex_digest(&fs::read(root.join("report.json")).unwrap())
    );
    validate_schema(
        include_str!("../schemas/render-report-v1.schema.json"),
        &result,
    );
    let mut contradictory_output = result.clone();
    contradictory_output["outputs"][0]["media_type"] = Value::String("application/json".to_owned());
    assert_schema_rejects(
        include_str!("../schemas/render-report-v1.schema.json"),
        &contradictory_output,
    );
    let persisted: Value =
        serde_json::from_slice(&fs::read(root.join("render/render-report.json")).unwrap()).unwrap();
    assert_eq!(persisted, result);
    let guide = fs::read_to_string(root.join("render/guide.md")).unwrap();
    assert!(guide.contains("Review the highlighted heading"));
    assert!(guide.contains("![Highlighted heading](001-focused.png)"));
    assert_eq!(
        fs::read(root.join("render/001-focused.png")).unwrap(),
        fs::read(root.join("evidence/003-capture-heading.focused.png")).unwrap()
    );
    for output in result["outputs"].as_array().unwrap() {
        let bytes = fs::read(root.join(output["path"].as_str().unwrap())).unwrap();
        assert_eq!(output["sha256"], crawlson::journey::hex_digest(&bytes));
    }
    assert!(!guide.contains(root.to_string_lossy().as_ref()));
}

#[test]
fn render_failure_emits_untriaged_findings_with_only_explicit_image_evidence() {
    let fixture = FakeAgentBrowser::compile();
    fixture.set_scenario("fail_text");
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert_eq!(run.status.code(), Some(1));
    let run_report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let root = PathBuf::from(run_report["run_directory"].as_str().unwrap());

    let output = render_command("crawlson", &root, &journey);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "findings_ready");
    assert_eq!(result["findings"], 1);
    validate_schema(
        include_str!("../schemas/render-report-v1.schema.json"),
        &result,
    );

    let findings: Value =
        serde_json::from_slice(&fs::read(root.join("render/findings.json")).unwrap()).unwrap();
    validate_schema(
        include_str!("../schemas/findings-v1.schema.json"),
        &findings,
    );
    let finding = &findings["findings"][0];
    assert_eq!(finding["severity"], "untriaged");
    assert_eq!(finding["severity_source"], "not_assessed");
    assert_eq!(finding["step"]["id"], "heading");
    assert!(finding["evidence"].as_array().unwrap().iter().any(|item| {
        item["kind"] == "focused_screenshot"
            && item["capture_step_id"] == "capture-heading"
            && item["association_source"] == "journey.evidence_for"
    }));

    for contradictory in [
        {
            let mut value = findings.clone();
            value["findings"][0]["kind"] = Value::String("url_mismatch".to_owned());
            value
        },
        {
            let mut value = findings.clone();
            value["findings"][0]["reproduction_steps"][0]["status"] =
                Value::String("blocked".to_owned());
            value
        },
        {
            let mut value = findings.clone();
            value["findings"][0]["reproduction_steps"][0]["kind"] =
                Value::String("capture".to_owned());
            value
        },
        {
            let mut value = findings.clone();
            value["findings"][0]["evidence"][0]["path"] =
                Value::String("../../report.json".to_owned());
            value
        },
        {
            let mut value = findings.clone();
            value["findings"][0]["evidence"][0]["capture_step_id"] =
                Value::String("capture-heading".to_owned());
            value
        },
        {
            let mut value = findings.clone();
            value["findings"][0]["evidence"]
                .as_array_mut()
                .unwrap()
                .retain(|item| item["kind"] != "focus_metadata");
            value
        },
    ] {
        assert_schema_rejects(
            include_str!("../schemas/findings-v1.schema.json"),
            &contradictory,
        );
    }
}

#[test]
fn render_blocked_and_no_guide_runs_are_explicitly_not_publishable() {
    for no_guide in [false, true] {
        let fixture = FakeAgentBrowser::compile();
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        if no_guide {
            let source = fs::read_to_string(&journey).unwrap().replace(
                "guide_instruction = \"Review the highlighted heading.\"\n",
                "",
            );
            fs::write(&journey, source).unwrap();
        }
        let authorization = if no_guide {
            vec!["--allow-origin", "http://127.0.0.1:4173"]
        } else {
            Vec::new()
        };
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &authorization,
        );
        let report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());
        let output = render_command("crawlson", &root, &journey);
        assert_eq!(
            output.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["status"], "not_publishable");
        assert_eq!(result["publishable"], false);
        validate_schema(
            include_str!("../schemas/render-report-v1.schema.json"),
            &result,
        );
        assert!(root.join("render/render-report.json").is_file());
        assert!(!root.join("render/guide.md").exists());
        assert!(!root.join("render/findings.json").exists());
    }
}

#[test]
fn render_rejects_journey_drift_artifact_tamper_and_path_escape_before_writing() {
    for fault in ["journey_drift", "artifact_tamper", "path_escape"] {
        let fixture = FakeAgentBrowser::compile();
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        assert!(run.status.success());
        let report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());

        match fault {
            "journey_drift" => {
                let source = fs::read_to_string(&journey).unwrap().replace(
                    "Review the highlighted heading.",
                    "Review the highlighted heading now.",
                );
                fs::write(&journey, source).unwrap();
            }
            "artifact_tamper" => {
                let focused = report["artifacts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|artifact| artifact["kind"] == "focused_screenshot")
                    .unwrap()["path"]
                    .as_str()
                    .unwrap();
                fs::write(root.join(focused), b"changed").unwrap();
            }
            "path_escape" => {
                let mut stored: Value =
                    serde_json::from_slice(&fs::read(root.join("report.json")).unwrap()).unwrap();
                stored["artifacts"][0]["path"] = Value::String("../outside".to_owned());
                fs::write(
                    root.join("report.json"),
                    format!("{}\n", serde_json::to_string_pretty(&stored).unwrap()),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }

        let output = render_command("crawlson", &root, &journey);
        assert_eq!(output.status.code(), Some(4), "{fault}");
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["status"], "error", "{fault}");
        validate_schema(
            include_str!("../schemas/render-report-v1.schema.json"),
            &result,
        );
        assert!(!root.join("render").exists(), "{fault}");
    }
}

#[test]
fn render_error_and_cleanup_failure_are_non_publishable() {
    for (scenario, reason) in [
        ("malformed", "run_error"),
        ("cleanup_fail", "cleanup_failed"),
    ] {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        assert_eq!(run.status.code(), Some(4));
        let report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());
        let output = render_command("crawlson", &root, &journey);
        assert_eq!(output.status.code(), Some(4));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["status"], "error");
        assert_eq!(result["publishable"], false);
        assert_eq!(result["reason"]["code"], reason);
        validate_schema(
            include_str!("../schemas/render-report-v1.schema.json"),
            &result,
        );
        assert!(root.join("render/render-report.json").is_file());
        assert!(!root.join("render/guide.md").exists());
        assert!(!root.join("render/findings.json").exists());
    }
}

#[test]
fn render_does_not_infer_image_evidence_and_rejects_incomplete_failed_capture() {
    for (scenario, explicit_image, expected_exit) in [("fail_text", false, 1), ("hidden", true, 4)]
    {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        if !explicit_image {
            let source = fs::read_to_string(&journey)
                .unwrap()
                .replace("evidence_for = [\"heading\"]\n", "");
            fs::write(&journey, source).unwrap();
        }
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        assert_eq!(run.status.code(), Some(1));
        let report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());
        let output = render_command("crawlson", &root, &journey);
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if expected_exit == 1 {
            let findings: Value =
                serde_json::from_slice(&fs::read(root.join("render/findings.json")).unwrap())
                    .unwrap();
            assert!(
                !findings["findings"][0]["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["kind"] == "focused_screenshot")
            );
        } else {
            assert!(!root.join("render/findings.json").exists());
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["status"], "error");
            assert_eq!(result["reason"]["code"], "run_incomplete");
        }
    }
}

#[cfg(unix)]
#[test]
fn render_rejects_even_in_tree_artifact_symlinks() {
    use std::os::unix::fs::symlink;

    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(run.status.success());
    let report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let raw_relative = report["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["kind"] == "raw_screenshot")
        .unwrap()["path"]
        .as_str()
        .unwrap();
    let raw = root.join(raw_relative);
    let real = raw.with_extension("real.png");
    fs::rename(&raw, &real).unwrap();
    symlink(real.file_name().unwrap(), &raw).unwrap();

    let output = render_command("crawlson", &root, &journey);
    assert_eq!(output.status.code(), Some(4));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["reason"]["code"], "artifact_path_escape");
    assert!(!root.join("render").exists());
}

#[test]
fn render_rejects_semantically_contradictory_publishable_reports() {
    let fixture = FakeAgentBrowser::compile();
    for fault in [
        "commands_empty",
        "reason_contradiction",
        "cleanup_error",
        "off_origin_capture",
        "missing_capture_url",
        "missing_text_digest",
    ] {
        fixture.set_scenario(if fault == "missing_text_digest" {
            "fail_text"
        } else {
            "pass"
        });
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        let mut report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());
        match fault {
            "commands_empty" => report["driver"]["commands"] = serde_json::json!([]),
            "reason_contradiction" => {
                report["reason"] = serde_json::json!({
                    "code": "checkpoint_failed",
                    "message": "contradictory pass"
                });
                report["execution_reason"] = report["reason"].clone();
            }
            "cleanup_error" => {
                report["cleanup"]["error"] = Value::String("impossible error".to_owned())
            }
            "off_origin_capture" => {
                report["steps"][2]["observation"]["observed_url"] =
                    Value::String("https://outside.example/".to_owned())
            }
            "missing_capture_url" => {
                report["steps"][2]["observation"]
                    .as_object_mut()
                    .unwrap()
                    .remove("observed_url");
            }
            "missing_text_digest" => {
                report["steps"][1]["observation"]
                    .as_object_mut()
                    .unwrap()
                    .remove("observed_text_sha256");
            }
            _ => unreachable!(),
        }
        write_json(root.join("report.json"), &report);
        let output = render_command("crawlson", &root, &journey);
        assert_eq!(output.status.code(), Some(4), "{fault}");
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["reason"]["code"], "report_invalid", "{fault}");
        assert!(!root.join("render").exists(), "{fault}");
    }
}

#[test]
fn render_anchors_focus_metadata_to_the_verified_png_dimensions() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(run.status.success());
    let mut report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let root = PathBuf::from(report["run_directory"].as_str().unwrap());

    let artifacts = report["artifacts"].as_array_mut().unwrap();
    let raw_index = artifacts
        .iter()
        .position(|artifact| artifact["kind"] == "raw_screenshot")
        .unwrap();
    let metadata_index = artifacts
        .iter()
        .position(|artifact| artifact["kind"] == "focus_metadata")
        .unwrap();
    let raw_path = root.join(artifacts[raw_index]["path"].as_str().unwrap());
    write_test_png(&raw_path, 640, 360);
    let raw_bytes = fs::read(&raw_path).unwrap();
    let raw_digest = crawlson::journey::hex_digest(&raw_bytes);
    artifacts[raw_index]["size_bytes"] = Value::from(raw_bytes.len() as u64);
    artifacts[raw_index]["sha256"] = Value::String(raw_digest.clone());

    let metadata_path = root.join(artifacts[metadata_index]["path"].as_str().unwrap());
    let mut metadata: Value = serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    metadata["source"]["size_bytes"] = Value::from(raw_bytes.len() as u64);
    metadata["source"]["sha256"] = Value::String(raw_digest);
    write_json(metadata_path.clone(), &metadata);
    let metadata_bytes = fs::read(&metadata_path).unwrap();
    artifacts[metadata_index]["size_bytes"] = Value::from(metadata_bytes.len() as u64);
    artifacts[metadata_index]["sha256"] =
        Value::String(crawlson::journey::hex_digest(&metadata_bytes));
    write_json(root.join("report.json"), &report);

    let output = render_command("crawlson", &root, &journey);
    assert_eq!(output.status.code(), Some(4));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["reason"]["code"], "artifact_tampered");
    assert!(!root.join("render").exists());
}

#[test]
fn render_preserves_in_run_safety_block_and_ignores_wall_clock_skew() {
    let fixture = FakeAgentBrowser::compile();
    for (scenario, expected_exit, expected_status) in [
        ("redirect", 3, "not_publishable"),
        ("pass", 0, "guide_ready"),
    ] {
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        let mut report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());
        if scenario == "pass" {
            report["finished_at_unix_ms"] = report["started_at_unix_ms"].clone();
            write_json(root.join("report.json"), &report);
        }
        let output = render_command("crawlson", &root, &journey);
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["status"], expected_status);
        if scenario == "redirect" {
            assert_eq!(result["reason"]["code"], "run_blocked");
        }
    }
}

#[test]
fn render_rejects_each_missing_registered_artifact_before_writing() {
    let fixture = FakeAgentBrowser::compile();
    for kind in [
        "raw_screenshot",
        "focused_screenshot",
        "focus_metadata",
        "trace",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        assert!(run.status.success());
        let report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(report["run_directory"].as_str().unwrap());
        let path = report["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["kind"] == kind)
            .unwrap()["path"]
            .as_str()
            .unwrap();
        fs::remove_file(root.join(path)).unwrap();
        let output = render_command("crawlson", &root, &journey);
        assert_eq!(output.status.code(), Some(4), "{kind}");
        assert!(!root.join("render").exists(), "{kind}");
    }
}

#[test]
fn render_preserves_conflicting_output_and_escapes_authored_markdown() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_journey(directory.path(), "Hello", false);
    let source = fs::read_to_string(&journey)
        .unwrap()
        .replace("title = \"Read home\"", "title = \"Read [home] #1\"")
        .replace(
            "guide_instruction = \"Review the highlighted heading.\"",
            "guide_instruction = \"Review [the](heading).\"",
        )
        .replace(
            "alt_text = \"Highlighted heading\"",
            "alt_text = \"Highlighted [heading]\"",
        );
    fs::write(&journey, source).unwrap();
    let run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(run.status.success());
    let report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let first = render_command("crawlson", &root, &journey);
    assert!(first.status.success());
    let guide_path = root.join("render/guide.md");
    let guide = fs::read_to_string(&guide_path).unwrap();
    assert!(guide.contains("Read \\[home\\] \\#1"));
    assert!(guide.contains("Review \\[the\\]\\(heading\\)\\."));
    assert!(guide.contains("![Highlighted \\[heading\\]](001-focused.png)"));
    fs::write(&guide_path, "conflicting user output\n").unwrap();
    let second = render_command("crawlson", &root, &journey);
    assert_eq!(second.status.code(), Some(4));
    assert_eq!(
        fs::read_to_string(&guide_path).unwrap(),
        "conflicting user output\n"
    );
}

fn write_json(path: PathBuf, value: &Value) {
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}

fn validate_schema(source: &str, instance: &Value) {
    let schema: Value = serde_json::from_str(source).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(instance)
        .unwrap();
}

fn assert_schema_rejects(source: &str, instance: &Value) {
    let schema: Value = serde_json::from_str(source).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(instance)
            .is_err()
    );
}

fn write_test_png(path: &Path, width: u32, height: u32) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[120, 130, 140, 255].repeat((width * height) as usize))
        .unwrap();
    writer.finish().unwrap();
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

fn render_command(name: &str, run_directory: &Path, journey: &Path) -> std::process::Output {
    Command::new(cargo_bin(name))
        .args(["--json", "render"])
        .arg(run_directory)
        .arg("--journey")
        .arg(journey)
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
schema_version = 2

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
evidence_for = ["heading"]
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
