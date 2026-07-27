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
fn follow_link_requires_the_exact_step_grant_before_driver_launch() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_follow_link_journey(directory.path(), "/complete");

    for grants in [
        Vec::<&str>::new(),
        vec!["--allow-action", "fixture.follow-link@1:other"],
    ] {
        let mut extra = vec!["--allow-origin", "http://127.0.0.1:4173"];
        extra.extend(grants);
        let output = run_command("crawlson", &journey, directory.path(), &fixture, &extra);
        assert_eq!(output.status.code(), Some(3));
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        validate_schema(
            include_str!("../schemas/run-report-v2.schema.json"),
            &report,
        );
        assert_eq!(report["schema_version"], 2);
        assert_eq!(report["outcome"], "blocked");
        assert_eq!(report["reason"]["code"], "action_authorization_mismatch");
        assert!(report["driver"]["commands"].as_array().unwrap().is_empty());
        assert!(report["artifacts"].as_array().unwrap().is_empty());
    }
    assert!(!fixture.call_log().exists());
}

#[test]
fn action_authorization_provenance_keeps_preflight_reports_renderable_and_versioned() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let journey = write_follow_link_journey(directory.path(), "/complete");
    let output = run_command("crawlson", &journey, directory.path(), &fixture, &[]);
    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_schema(
        include_str!("../schemas/run-report-v2.schema.json"),
        &report,
    );
    assert_eq!(report["reason"]["code"], "target_authorization_missing");
    assert_eq!(
        report["action_authorization"]["required"],
        serde_json::json!(["fixture.follow-link@1:follow-continue"])
    );
    assert!(
        report["action_authorization"]["granted"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let rendered = render_command("crawlson", &root, &journey);
    assert_eq!(rendered.status.code(), Some(3));
    let rendered: Value = serde_json::from_slice(&rendered.stdout).unwrap();
    assert_eq!(rendered["status"], "not_publishable");

    let read_only = write_journey(directory.path(), "Hello", false);
    let output = run_command(
        "crawlson",
        &read_only,
        directory.path(),
        &fixture,
        &[
            "--allow-origin",
            "http://127.0.0.1:4173",
            "--allow-action",
            "fixture.read-only@1:unexpected",
        ],
    );
    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_schema(
        include_str!("../schemas/run-report-v1.schema.json"),
        &report,
    );
    assert_eq!(report["reason"]["code"], "action_authorization_unexpected");
    assert!(report.get("action_authorization").is_none());

    let action = write_follow_link_journey(directory.path(), "/complete");
    let output = run_command(
        "crawlson",
        &action,
        directory.path(),
        &fixture,
        &[
            "--allow-origin",
            "http://127.0.0.1:4173",
            "--allow-action",
            "secret malformed grant",
        ],
    );
    assert_eq!(output.status.code(), Some(3));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret malformed grant"));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_schema(
        include_str!("../schemas/run-report-v2.schema.json"),
        &report,
    );
    assert_eq!(report["reason"]["code"], "action_authorization_invalid");
    assert!(
        report["action_authorization"]["granted"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!fixture.call_log().exists());

    let fixture = FakeAgentBrowser::compile();
    let capture_only = write_journey(directory.path(), "Hello", false);
    let source = fs::read_to_string(&capture_only)
        .unwrap()
        .replace("schema_version = 2", "schema_version = 3");
    fs::write(&capture_only, source).unwrap();
    let output = run_command(
        "crawlson",
        &capture_only,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert_eq!(output.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_schema(
        include_str!("../schemas/run-report-v2.schema.json"),
        &report,
    );
    assert!(
        report["action_authorization"]["required"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        report["action_authorization"]["granted"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let rendered = render_command("crawlson", &root, &capture_only);
    assert_eq!(rendered.status.code(), Some(0));
}

#[test]
fn follow_link_executes_once_and_renders_only_verified_action_evidence() {
    for name in ["crawlson", "clson"] {
        let fixture = FakeAgentBrowser::compile();
        let directory = tempfile::tempdir().unwrap();
        let journey = write_follow_link_journey(directory.path(), "/complete");
        let output = run_command(
            name,
            &journey,
            directory.path(),
            &fixture,
            &[
                "--allow-origin",
                "http://127.0.0.1:4173",
                "--allow-action",
                "fixture.follow-link@1:follow-continue",
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        validate_schema(
            include_str!("../schemas/run-report-v2.schema.json"),
            &report,
        );
        assert_eq!(report["schema_version"], 2);
        assert_eq!(report["outcome"], "passed");
        let action = report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == "follow-continue")
            .unwrap();
        assert_eq!(action["kind"], "follow_link");
        assert_eq!(action["status"], "passed");
        assert_eq!(action["observation"]["action_state"], "effect_verified");
        assert_eq!(
            action["observation"]["target_href"],
            "http://127.0.0.1:4173/complete"
        );
        assert!(
            action["observation"]["action_command_sequence"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(
            report["action_authorization"]["required"],
            serde_json::json!(["fixture.follow-link@1:follow-continue"])
        );
        let calls = fs::read_to_string(fixture.call_log()).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("\tclick\t"))
                .count(),
            1
        );
        assert!(calls.contains(":is(#action-button):is(a[href=\"/complete\"])"));
        assert!(!calls.contains("\tclick\t#action-button\t"));

        let run_root = PathBuf::from(report["run_directory"].as_str().unwrap());
        let render = render_command(name, &run_root, &journey);
        assert_eq!(
            render.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&render.stdout)
        );
        let rendered: Value = serde_json::from_slice(&render.stdout).unwrap();
        assert_eq!(rendered["status"], "guide_ready");
        let guide = fs::read_to_string(run_root.join("render/guide.md")).unwrap();
        assert!(guide.contains("executed this highlighted link action once"));
        assert!(guide.contains("verified its exact declared same-origin destination"));
    }
}

#[test]
fn follow_link_failures_and_unknown_effects_are_honest() {
    for (scenario, extra_timeout, exit, outcome, code, action_state) in [
        (
            "hidden_link",
            false,
            1,
            "failed",
            "checkpoint_failed",
            "not_attempted",
        ),
        (
            "disabled",
            false,
            1,
            "failed",
            "checkpoint_failed",
            "not_attempted",
        ),
        (
            "href_missing",
            false,
            1,
            "failed",
            "checkpoint_failed",
            "not_attempted",
        ),
        (
            "href_mismatch",
            false,
            1,
            "failed",
            "checkpoint_failed",
            "not_attempted",
        ),
        (
            "href_off_origin",
            false,
            3,
            "blocked",
            "origin_not_authorized",
            "not_attempted",
        ),
        (
            "click_confirmation",
            false,
            4,
            "error",
            "driver_confirmation_required",
            "not_attempted",
        ),
        (
            "click_error",
            false,
            4,
            "error",
            "action_effect_unknown",
            "effect_unknown",
        ),
        (
            "click_response_mismatch",
            false,
            4,
            "error",
            "action_effect_unknown",
            "effect_unknown",
        ),
        (
            "click_unknown",
            false,
            4,
            "error",
            "action_effect_unknown",
            "effect_unknown",
        ),
        (
            "click_timeout",
            true,
            4,
            "error",
            "action_effect_unknown",
            "effect_unknown",
        ),
        (
            "click_post_origin_escape",
            false,
            3,
            "blocked",
            "origin_not_authorized",
            "driver_acknowledged",
        ),
        (
            "click_wrong_destination",
            false,
            1,
            "failed",
            "checkpoint_failed",
            "driver_acknowledged",
        ),
    ] {
        let fixture = FakeAgentBrowser::compile();
        fixture.set_scenario(scenario);
        let directory = tempfile::tempdir().unwrap();
        let journey = write_follow_link_journey(directory.path(), "/complete");
        let mut extra = vec![
            "--allow-origin",
            "http://127.0.0.1:4173",
            "--allow-action",
            "fixture.follow-link@1:follow-continue",
        ];
        if extra_timeout {
            extra.extend(["--action-timeout-seconds", "1"]);
        }
        let output = run_command("crawlson", &journey, directory.path(), &fixture, &extra);
        assert_eq!(
            output.status.code(),
            Some(exit),
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["outcome"], outcome, "{scenario}");
        assert_eq!(report["reason"]["code"], code, "{scenario}");
        let action = report["steps"].as_array().unwrap().last().unwrap();
        assert_eq!(action["id"], "follow-continue", "{scenario}");
        assert_eq!(
            action["observation"]["action_state"], action_state,
            "{scenario}"
        );
        assert_eq!(report["cleanup"]["status"], "passed", "{scenario}");

        if outcome == "failed" {
            let root = PathBuf::from(report["run_directory"].as_str().unwrap());
            let rendered = render_command("crawlson", &root, &journey);
            assert_eq!(
                rendered.status.code(),
                Some(1),
                "{scenario}: {}",
                String::from_utf8_lossy(&rendered.stdout)
            );
            let rendered: Value = serde_json::from_slice(&rendered.stdout).unwrap();
            assert_eq!(rendered["status"], "findings_ready", "{scenario}");
            let findings: Value =
                serde_json::from_slice(&fs::read(root.join("render/findings.json")).unwrap())
                    .unwrap();
            validate_schema(
                include_str!("../schemas/findings-v2.schema.json"),
                &findings,
            );
            let finding = &findings["findings"][0];
            let expected_kind = match scenario {
                "hidden_link" => "link_not_visible",
                "disabled" => "link_not_enabled",
                "href_missing" => "link_target_invalid",
                "href_mismatch" => "link_destination_mismatch",
                "click_wrong_destination" => "link_postcondition_mismatch",
                _ => unreachable!(),
            };
            assert_eq!(finding["kind"], expected_kind, "{scenario}");
            if scenario == "click_wrong_destination" {
                assert_eq!(finding["checkpoint"]["expected"], "/complete");
                assert_eq!(finding["checkpoint"]["observed_path"], "/unexpected");
                assert_eq!(finding["checkpoint"]["action_state"], "driver_acknowledged");
            }
        }
    }
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
        (
            "confirmation_required",
            vec![],
            "driver_confirmation_required",
        ),
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

    for id in [".leading", "_leading", "-leading"] {
        let mut leading_punctuation = document.clone();
        leading_punctuation["journey"]["id"] = Value::String(id.to_owned());
        validator.validate(&leading_punctuation).unwrap();
    }

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

#[test]
fn guide_collection_build_and_check_are_deterministic_alias_equivalent() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let read_journey = write_journey(directory.path(), "Hello", false);
    let read_run = run_command(
        "crawlson",
        &read_journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(read_run.status.success());
    let read_report: Value = serde_json::from_slice(&read_run.stdout).unwrap();
    let read_root = PathBuf::from(read_report["run_directory"].as_str().unwrap());

    let action_journey = write_follow_link_journey(directory.path(), "/complete");
    let action_run = run_command(
        "crawlson",
        &action_journey,
        directory.path(),
        &fixture,
        &[
            "--allow-origin",
            "http://127.0.0.1:4173",
            "--allow-action",
            "fixture.follow-link@1:follow-continue",
        ],
    );
    assert!(action_run.status.success());
    let action_report: Value = serde_json::from_slice(&action_run.stdout).unwrap();
    let action_root = PathBuf::from(action_report["run_directory"].as_str().unwrap());

    let manifest = write_collection_manifest(
        directory.path(),
        &[
            ("read-home", 10, &read_root, &read_journey),
            ("follow-continue", 20, &action_root, &action_journey),
        ],
    );
    let manifest_document: toml::Value =
        toml::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    validate_schema(
        include_str!("../schemas/guide-collection-manifest-v1.schema.json"),
        &serde_json::to_value(manifest_document).unwrap(),
    );
    let output = directory.path().join("guide-site");
    let missing = collection_command("crawlson", "check", &manifest, &output);
    assert_eq!(missing.status.code(), Some(3));
    let missing: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing["status"], "not_publishable");
    assert_eq!(missing["diagnostics"][0]["code"], "output_missing");
    assert!(!output.exists());
    let build = collection_command("crawlson", "build", &manifest, &output);
    assert_eq!(
        build.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );
    assert!(build.stderr.is_empty());
    let report: Value = serde_json::from_slice(&build.stdout).unwrap();
    assert_eq!(report["status"], "ready");
    assert_eq!(report["publishable"], true);
    assert_eq!(report["summary"]["guides"], 2);
    validate_schema(
        include_str!("../schemas/guide-collection-report-v1.schema.json"),
        &report,
    );
    let application: Value =
        serde_json::from_slice(&fs::read(output.join("guide-collection.json")).unwrap()).unwrap();
    validate_schema(
        include_str!("../schemas/guide-collection-v1.schema.json"),
        &application,
    );
    assert_eq!(
        application["collection"]["manifest_sha256"],
        report["collection"]["manifest_sha256"]
    );
    assert_eq!(
        application["collection"]["snapshot_sha256"],
        report["collection"]["snapshot_sha256"]
    );
    assert!(output.join("index.md").is_file());
    assert!(output.join("topics/basics/index.md").is_file());
    assert!(output.join("topics/basics/read-home/index.md").is_file());
    assert!(
        output
            .join("topics/basics/follow-continue/index.md")
            .is_file()
    );
    assert!(
        fs::read_to_string(output.join("index.md"))
            .unwrap()
            .contains("topics/basics/index.md")
    );
    assert!(
        fs::read_to_string(output.join("topics/basics/read-home/index.md"))
            .unwrap()
            .contains("../follow-continue/index.md")
    );
    let topic_index = fs::read_to_string(output.join("topics/basics/index.md")).unwrap();
    assert!(topic_index.contains("1. [Read home](read-home/index.md)"));
    assert!(topic_index.contains("2. [Follow Continue](follow-continue/index.md)"));
    assert!(!topic_index.contains("10. ["));
    assert!(!topic_index.contains("20. ["));
    let root_index = fs::read_to_string(output.join("index.md")).unwrap();
    assert!(root_index.contains("Audience: visitors"));
    let direct_guide = fs::read_to_string(output.join("topics/basics/read-home/index.md")).unwrap();
    assert!(direct_guide.contains("Audience: visitors"));
    assert!(direct_guide.contains("[← Basics](../index.md)"));

    let app_guides = application["topics"][0]["guides"].as_array().unwrap();
    assert_eq!(app_guides.len(), 2);
    for app_guide in app_guides {
        let page = app_guide["page"].as_str().unwrap();
        let page_bytes = fs::read(output.join(page)).unwrap();
        assert_eq!(app_guide["page_size_bytes"], page_bytes.len() as u64);
        assert_eq!(
            app_guide["page_sha256"],
            crawlson::journey::hex_digest(&page_bytes)
        );
    }
    let read_app = &app_guides[0];
    assert_eq!(read_app["journey"]["id"], "fixture.read-home");
    assert_eq!(read_app["steps"].as_array().unwrap().len(), 1);
    let read_step = &read_app["steps"][0];
    assert_eq!(read_step["id"], "capture-heading");
    assert_eq!(read_step["number"], 1);
    assert_eq!(read_step["title"], "Capture the heading");
    assert_eq!(read_step["instruction"], "Review the highlighted heading.");
    assert_eq!(read_step["claim_type"], "observed_next_action");
    assert_eq!(read_step["alt_text"], "Highlighted heading");
    assert!(
        read_step["claim"]
            .as_str()
            .unwrap()
            .contains("does not claim that action was executed")
    );
    assert_eq!(read_app["images"][0], read_step["image"]);

    let action_app = &app_guides[1];
    assert_eq!(action_app["journey"]["id"], "fixture.follow-link");
    assert_eq!(action_app["steps"].as_array().unwrap().len(), 1);
    assert_eq!(action_app["steps"][0]["id"], "follow-continue");
    assert_eq!(
        action_app["steps"][0]["claim_type"],
        "executed_and_verified"
    );
    assert!(
        action_app["steps"][0]["claim"]
            .as_str()
            .unwrap()
            .contains("executed this highlighted link action once")
    );

    let read_report_sha =
        crawlson::journey::hex_digest(&fs::read(read_root.join("report.json")).unwrap());
    assert_eq!(read_app["report_sha256"], read_report_sha);
    for path in [
        read_app["page"].as_str().unwrap(),
        read_step["image"]["path"].as_str().unwrap(),
    ] {
        let record = report["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["path"] == path)
            .unwrap();
        assert_eq!(record["topic_id"], "basics");
        assert_eq!(record["entry"], "read-home");
        assert_eq!(record["journey_id"], "fixture.read-home");
        assert_eq!(record["report_sha256"], read_report_sha);
        let bytes = fs::read(output.join(path)).unwrap();
        assert_eq!(record["size_bytes"], bytes.len() as u64);
        assert_eq!(record["sha256"], crawlson::journey::hex_digest(&bytes));
    }
    let read_focused = read_report["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["kind"] == "focused_screenshot")
        .unwrap()["path"]
        .as_str()
        .unwrap();
    assert_eq!(
        fs::read(output.join("topics/basics/read-home/001-focused.png")).unwrap(),
        fs::read(read_root.join(read_focused)).unwrap()
    );
    assert!(!read_root.join("render").exists());
    assert!(!action_root.join("render").exists());

    let before = snapshot_directory(&output);
    let check = collection_command("clson", "check", &manifest, &output);
    assert_eq!(check.status.code(), Some(0));
    assert_eq!(build.stdout, check.stdout);
    assert_eq!(before, snapshot_directory(&output));
    let second = collection_command("clson", "build", &manifest, &output);
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(build.stdout, second.stdout);
    assert_eq!(before, snapshot_directory(&output));
}

#[test]
fn guide_collection_surfaces_findings_without_emitting_a_partial_public_index() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    let pass_journey = write_journey(directory.path(), "Hello", false);
    let pass_run = run_command(
        "crawlson",
        &pass_journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    let pass_report: Value = serde_json::from_slice(&pass_run.stdout).unwrap();
    let pass_root = PathBuf::from(pass_report["run_directory"].as_str().unwrap());

    let fail_journey = directory.path().join("failure.toml");
    let failure_source = fs::read_to_string(&pass_journey)
        .unwrap()
        .replace("fixture.read-home", "fixture.fail-home")
        .replace("title = \"Read home\"", "title = \"Find broken heading\"")
        .replace("expected = \"Hello\"", "expected = \"Missing\"");
    fs::write(&fail_journey, failure_source).unwrap();
    let fail_run = run_command(
        "crawlson",
        &fail_journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert_eq!(fail_run.status.code(), Some(1));
    let fail_report: Value = serde_json::from_slice(&fail_run.stdout).unwrap();
    let fail_root = PathBuf::from(fail_report["run_directory"].as_str().unwrap());

    let blocked_journey = directory.path().join("blocked.toml");
    let blocked_source = fs::read_to_string(&pass_journey)
        .unwrap()
        .replace("fixture.read-home", "fixture.blocked-home")
        .replace("title = \"Read home\"", "title = \"Blocked home\"");
    fs::write(&blocked_journey, blocked_source).unwrap();
    let blocked_run = run_command(
        "crawlson",
        &blocked_journey,
        directory.path(),
        &fixture,
        &[],
    );
    assert_eq!(blocked_run.status.code(), Some(3));
    let blocked_report: Value = serde_json::from_slice(&blocked_run.stdout).unwrap();
    let blocked_root = PathBuf::from(blocked_report["run_directory"].as_str().unwrap());

    let manifest = write_collection_manifest(
        directory.path(),
        &[
            ("read-home", 10, &pass_root, &pass_journey),
            ("broken-heading", 20, &fail_root, &fail_journey),
            ("blocked-home", 30, &blocked_root, &blocked_journey),
        ],
    );
    let output = directory.path().join("review-site");
    let build = collection_command("crawlson", "build", &manifest, &output);
    assert_eq!(
        build.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let report: Value = serde_json::from_slice(&build.stdout).unwrap();
    assert_eq!(report["status"], "not_publishable");
    assert_eq!(report["publishable"], false);
    assert_eq!(report["summary"]["guides"], 1);
    assert_eq!(report["summary"]["findings"], 1);
    assert_eq!(report["summary"]["unavailable"], 1);
    assert_eq!(report["summary"]["errors"], 0);
    assert_eq!(report["entries"][0]["status"], "guide_ready");
    assert_eq!(report["entries"][1]["status"], "findings_ready");
    assert_eq!(report["entries"][2]["status"], "not_publishable");
    validate_schema(
        include_str!("../schemas/guide-collection-report-v1.schema.json"),
        &report,
    );
    assert!(!output.join("index.md").exists());
    assert!(!output.join("guide-collection.json").exists());
    assert!(output.join("review/index.md").is_file());
    let findings_root = output.join("review/basics/broken-heading");
    assert!(findings_root.join("render/findings.json").is_file());
    assert!(findings_root.join("render/findings.md").is_file());
    assert!(findings_root.join("report.json").is_file());
    assert!(findings_root.join("evidence/trace.json").is_file());
    assert!(
        fs::read_to_string(output.join("review/index.md"))
            .unwrap()
            .contains("basics/broken-heading/render/findings.md")
    );
    let review_index = fs::read_to_string(output.join("review/index.md")).unwrap();
    assert!(review_index.contains("Read home** — verified, but withheld"));
    assert!(review_index.contains("Find broken heading** — 1 finding(s)"));
    assert!(review_index.contains("Blocked home** — unavailable"));
    let check = collection_command("clson", "check", &manifest, &output);
    assert_eq!(check.status.code(), Some(3));
    assert_eq!(build.stdout, check.stdout);
}

#[test]
fn guide_collection_preserves_blocked_and_tampered_outcomes() {
    let fixture = FakeAgentBrowser::compile();
    for (fault, expected_reason) in [
        ("blocked", "run_blocked"),
        ("tampered", "artifact_tampered"),
        ("run-error", "run_error"),
        ("cleanup-error", "cleanup_failed"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        fixture.set_scenario(match fault {
            "run-error" => "malformed",
            "cleanup-error" => "cleanup_fail",
            _ => "pass",
        });
        let authorization = if fault == "blocked" {
            Vec::new()
        } else {
            vec!["--allow-origin", "http://127.0.0.1:4173"]
        };
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &authorization,
        );
        let run_report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(run_report["run_directory"].as_str().unwrap());
        if fault == "tampered" {
            let focused = run_report["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|artifact| artifact["kind"] == "focused_screenshot")
                .unwrap()["path"]
                .as_str()
                .unwrap();
            fs::write(root.join(focused), b"tampered").unwrap();
        }
        let manifest =
            write_collection_manifest(directory.path(), &[("read-home", 10, &root, &journey)]);
        let output = directory.path().join("collection");
        let build = collection_command("crawlson", "build", &manifest, &output);
        if fault == "blocked" {
            assert_eq!(build.status.code(), Some(3));
            let report: Value = serde_json::from_slice(&build.stdout).unwrap();
            assert_eq!(report["status"], "not_publishable");
            assert_eq!(report["summary"]["unavailable"], 1);
            assert_eq!(report["entries"][0]["reason_code"], expected_reason);
            validate_schema(
                include_str!("../schemas/guide-collection-report-v1.schema.json"),
                &report,
            );
            assert!(output.join("review/index.md").is_file());
            assert!(!output.join("index.md").exists());
        } else {
            assert_eq!(build.status.code(), Some(4));
            let report: Value = serde_json::from_slice(&build.stdout).unwrap();
            assert_eq!(report["status"], "error");
            assert_eq!(report["diagnostics"][0]["code"], expected_reason);
            assert_eq!(report["diagnostics"][0]["entry"], "read-home");
            assert_eq!(report["entries"][0]["reason_code"], expected_reason);
            validate_schema(
                include_str!("../schemas/guide-collection-report-v1.schema.json"),
                &report,
            );
            assert!(!output.exists());
        }
    }
}

#[test]
fn guide_collection_aggregates_guide_ready_and_error_entries_without_output() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    fixture.set_scenario("pass");
    let pass_journey = write_journey(directory.path(), "Hello", false);
    let pass_run = run_command(
        "crawlson",
        &pass_journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(pass_run.status.success());
    let pass_report: Value = serde_json::from_slice(&pass_run.stdout).unwrap();
    let pass_root = PathBuf::from(pass_report["run_directory"].as_str().unwrap());

    let error_journey = directory.path().join("error.toml");
    let error_source = fs::read_to_string(&pass_journey)
        .unwrap()
        .replace("fixture.read-home", "fixture.error-home")
        .replace("title = \"Read home\"", "title = \"Error home\"");
    fs::write(&error_journey, error_source).unwrap();
    fixture.set_scenario("malformed");
    let error_run = run_command(
        "crawlson",
        &error_journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert_eq!(error_run.status.code(), Some(4));
    let error_report: Value = serde_json::from_slice(&error_run.stdout).unwrap();
    let error_root = PathBuf::from(error_report["run_directory"].as_str().unwrap());

    let manifest = write_collection_manifest(
        directory.path(),
        &[
            ("read-home", 10, &pass_root, &pass_journey),
            ("error-home", 20, &error_root, &error_journey),
        ],
    );
    let output = directory.path().join("error-collection");
    let build = collection_command("crawlson", "build", &manifest, &output);
    assert_eq!(build.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&build.stdout).unwrap();
    assert_eq!(report["status"], "error");
    assert_eq!(report["publishable"], false);
    assert_eq!(report["summary"]["guides"], 1);
    assert_eq!(report["summary"]["errors"], 1);
    assert_eq!(report["entries"].as_array().unwrap().len(), 2);
    assert_eq!(report["entries"][0]["status"], "guide_ready");
    assert_eq!(report["entries"][1]["status"], "error");
    assert_eq!(report["entries"][1]["reason_code"], "run_error");
    assert_eq!(report["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(report["diagnostics"][0]["code"], "run_error");
    assert_eq!(report["diagnostics"][0]["entry"], "error-home");
    assert!(report["outputs"].as_array().unwrap().is_empty());
    validate_schema(
        include_str!("../schemas/guide-collection-report-v1.schema.json"),
        &report,
    );
    assert!(!output.exists());
}

#[test]
fn guide_collection_check_treats_status_shape_transitions_as_stale_and_read_only() {
    let fixture = FakeAgentBrowser::compile();
    let directory = tempfile::tempdir().unwrap();
    fixture.set_scenario("pass");
    let journey = write_journey(directory.path(), "Hello", false);
    let ready_run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert!(ready_run.status.success());
    let ready_report: Value = serde_json::from_slice(&ready_run.stdout).unwrap();
    let ready_root = PathBuf::from(ready_report["run_directory"].as_str().unwrap());
    let ready_manifest = write_collection_manifest(
        directory.path(),
        &[("read-home", 10, &ready_root, &journey)],
    );
    let ready_output = directory.path().join("ready-output");
    assert!(
        collection_command("crawlson", "build", &ready_manifest, &ready_output)
            .status
            .success()
    );
    let ready_snapshot = snapshot_directory(&ready_output);

    let blocked_run = run_command("crawlson", &journey, directory.path(), &fixture, &[]);
    assert_eq!(blocked_run.status.code(), Some(3));
    let blocked_report: Value = serde_json::from_slice(&blocked_run.stdout).unwrap();
    let blocked_root = PathBuf::from(blocked_report["run_directory"].as_str().unwrap());
    let blocked_manifest = write_collection_manifest(
        directory.path(),
        &[("read-home", 10, &blocked_root, &journey)],
    );
    let blocked_check = collection_command("crawlson", "check", &blocked_manifest, &ready_output);
    assert_eq!(blocked_check.status.code(), Some(3));
    let blocked_check: Value = serde_json::from_slice(&blocked_check.stdout).unwrap();
    assert_eq!(blocked_check["status"], "not_publishable");
    assert!(
        blocked_check["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["code"] == "stale_output")
    );
    assert_eq!(ready_snapshot, snapshot_directory(&ready_output));

    fixture.set_scenario("fail_text");
    let findings_run = run_command(
        "crawlson",
        &journey,
        directory.path(),
        &fixture,
        &["--allow-origin", "http://127.0.0.1:4173"],
    );
    assert_eq!(findings_run.status.code(), Some(1));
    let findings_report: Value = serde_json::from_slice(&findings_run.stdout).unwrap();
    let findings_root = PathBuf::from(findings_report["run_directory"].as_str().unwrap());
    let findings_manifest = write_collection_manifest(
        directory.path(),
        &[("read-home", 10, &findings_root, &journey)],
    );
    let findings_check = collection_command("crawlson", "check", &findings_manifest, &ready_output);
    assert_eq!(findings_check.status.code(), Some(3));
    let findings_check: Value = serde_json::from_slice(&findings_check.stdout).unwrap();
    assert_eq!(findings_check["status"], "not_publishable");
    assert!(
        findings_check["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["code"] == "stale_output")
    );
    assert_eq!(ready_snapshot, snapshot_directory(&ready_output));

    let review_output = directory.path().join("review-output");
    assert_eq!(
        collection_command("crawlson", "build", &findings_manifest, &review_output)
            .status
            .code(),
        Some(1)
    );
    let review_snapshot = snapshot_directory(&review_output);
    let ready_manifest = write_collection_manifest(
        directory.path(),
        &[("read-home", 10, &ready_root, &journey)],
    );
    let ready_check = collection_command("crawlson", "check", &ready_manifest, &review_output);
    assert_eq!(ready_check.status.code(), Some(3));
    let ready_check: Value = serde_json::from_slice(&ready_check.stdout).unwrap();
    assert_eq!(ready_check["status"], "not_publishable");
    assert!(
        ready_check["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["code"] == "stale_output")
    );
    assert_eq!(review_snapshot, snapshot_directory(&review_output));
}

#[test]
fn guide_collection_check_detects_stale_dead_orphaned_and_unindexed_output_read_only() {
    let fixture = FakeAgentBrowser::compile();
    for fault in ["stale", "changed", "dead", "orphan", "unindexed"] {
        let directory = tempfile::tempdir().unwrap();
        let journey = write_journey(directory.path(), "Hello", false);
        let run = run_command(
            "crawlson",
            &journey,
            directory.path(),
            &fixture,
            &["--allow-origin", "http://127.0.0.1:4173"],
        );
        let run_report: Value = serde_json::from_slice(&run.stdout).unwrap();
        let root = PathBuf::from(run_report["run_directory"].as_str().unwrap());
        let manifest =
            write_collection_manifest(directory.path(), &[("read-home", 10, &root, &journey)]);
        let output = directory.path().join("collection");
        assert!(
            collection_command("crawlson", "build", &manifest, &output)
                .status
                .success()
        );
        match fault {
            "stale" => {
                let source = fs::read_to_string(&manifest)
                    .unwrap()
                    .replace("Verified help", "Current verified help");
                fs::write(&manifest, source).unwrap();
            }
            "changed" => {
                fs::write(
                    output.join("topics/basics/read-home/001-focused.png"),
                    b"changed image bytes",
                )
                .unwrap();
            }
            "dead" => {
                let index = output.join("index.md");
                let source = fs::read_to_string(&index)
                    .unwrap()
                    .replace("topics/basics/index.md", "topics/missing/index.md");
                fs::write(index, source).unwrap();
            }
            "orphan" => {
                fs::write(output.join("orphan.png"), b"not a registered image").unwrap();
            }
            "unindexed" => {
                let topic = output.join("topics/basics/index.md");
                let source = fs::read_to_string(&topic)
                    .unwrap()
                    .replace("1. [Read home](read-home/index.md)\n", "");
                fs::write(topic, source).unwrap();
            }
            _ => unreachable!(),
        }
        let before = snapshot_directory(&output);
        let check = collection_command("crawlson", "check", &manifest, &output);
        let report: Value = serde_json::from_slice(&check.stdout).unwrap();
        validate_schema(
            include_str!("../schemas/guide-collection-report-v1.schema.json"),
            &report,
        );
        match fault {
            "stale" => {
                assert_eq!(check.status.code(), Some(3));
                assert!(
                    report["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["code"] == "stale_output")
                );
            }
            "changed" => {
                assert_eq!(check.status.code(), Some(4));
                assert!(
                    report["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["code"] == "changed_output")
                );
            }
            "dead" => {
                assert_eq!(check.status.code(), Some(4));
                assert!(
                    report["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["code"] == "dead_link")
                );
            }
            "orphan" => {
                assert_eq!(check.status.code(), Some(4));
                assert!(
                    report["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["code"] == "orphan_image")
                );
            }
            "unindexed" => {
                assert_eq!(check.status.code(), Some(4));
                assert!(
                    report["diagnostics"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["code"] == "missing_index_entry")
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(before, snapshot_directory(&output));
        let rebuild = collection_command("crawlson", "build", &manifest, &output);
        assert_eq!(rebuild.status.code(), Some(4));
        assert_eq!(before, snapshot_directory(&output));
    }
}

#[test]
fn guide_collection_rejects_ambiguous_or_escaping_manifests_before_output() {
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
    let run_report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let root = PathBuf::from(run_report["run_directory"].as_str().unwrap());
    for (fault, expected_code) in [
        ("duplicate-key", "guide_key_duplicate"),
        ("duplicate-journey", "journey_identity_duplicate"),
        ("escape", "manifest_path_invalid"),
        ("unknown", "manifest_invalid"),
        ("repeated-slash", "manifest_path_invalid"),
        ("trailing-slash", "manifest_path_invalid"),
        ("reserved-path", "manifest_path_invalid"),
        ("trailing-dot-path", "manifest_path_invalid"),
        ("illegal-character-path", "manifest_path_invalid"),
        ("index-guide-key", "manifest_invalid"),
        ("trailing-dot-id", "manifest_invalid"),
        ("reserved-con", "manifest_invalid"),
        ("reserved-prn", "manifest_invalid"),
        ("reserved-aux", "manifest_invalid"),
        ("reserved-nul", "manifest_invalid"),
        ("reserved-com1", "manifest_invalid"),
        ("reserved-lpt9", "manifest_invalid"),
        ("reserved-extension", "manifest_invalid"),
    ] {
        let manifest =
            write_collection_manifest(directory.path(), &[("read-home", 10, &root, &journey)]);
        let mut source = fs::read_to_string(&manifest).unwrap();
        let run_relative = portable_relative(directory.path(), &root);
        match fault {
            "duplicate-key" => source.push_str(&format!(
                "\n[[topics.guides]]\nkey = \"read-home\"\norder = 20\nrun = \"{}\"\njourney = \"{}\"\n",
                portable_relative(directory.path(), &root),
                portable_relative(directory.path(), &journey)
            )),
            "duplicate-journey" => source.push_str(&format!(
                "\n[[topics.guides]]\nkey = \"read-home-again\"\norder = 20\nrun = \"{}\"\njourney = \"{}\"\n",
                portable_relative(directory.path(), &root),
                portable_relative(directory.path(), &journey)
            )),
            "escape" => source = source.replace(
                &format!("journey = \"{}\"", portable_relative(directory.path(), &journey)),
                "journey = \"../outside.toml\"",
            ),
            "unknown" => source.push_str("\nunknown = true\n"),
            "repeated-slash" => {
                let repeated = run_relative.replacen('/', "//", 1);
                source = source.replace(
                    &format!("run = \"{run_relative}\""),
                    &format!("run = \"{repeated}\""),
                );
            }
            "trailing-slash" => {
                source = source.replace(
                    &format!("run = \"{run_relative}\""),
                    &format!("run = \"{run_relative}/\""),
                );
            }
            "reserved-path" => {
                source = source.replace(
                    &format!("run = \"{run_relative}\""),
                    "run = \"con/report.json\"",
                );
            }
            "trailing-dot-path" => {
                source = source.replace(
                    &format!("run = \"{run_relative}\""),
                    "run = \"runs./report.json\"",
                );
            }
            "illegal-character-path" => {
                source = source.replace(
                    &format!("run = \"{run_relative}\""),
                    "run = \"runs?/report.json\"",
                );
            }
            "index-guide-key" => {
                source = source.replace("key = \"read-home\"", "key = \"index.md\"");
            }
            "trailing-dot-id" => {
                source = source.replace("id = \"fixture-help\"", "id = \"fixture-help.\"");
            }
            "reserved-con" => {
                source = source.replace("id = \"fixture-help\"", "id = \"con\"");
            }
            "reserved-prn" => {
                source = source.replace("id = \"fixture-help\"", "id = \"prn\"");
            }
            "reserved-aux" => {
                source = source.replace("id = \"fixture-help\"", "id = \"aux\"");
            }
            "reserved-nul" => {
                source = source.replace("id = \"fixture-help\"", "id = \"nul\"");
            }
            "reserved-com1" => {
                source = source.replace("id = \"fixture-help\"", "id = \"com1\"");
            }
            "reserved-lpt9" => {
                source = source.replace("id = \"fixture-help\"", "id = \"lpt9\"");
            }
            "reserved-extension" => {
                source = source.replace("id = \"fixture-help\"", "id = \"con.txt\"");
            }
            _ => unreachable!(),
        }
        if !matches!(fault, "duplicate-key" | "duplicate-journey") {
            let document: toml::Value = toml::from_str(&source).unwrap();
            assert_schema_rejects(
                include_str!("../schemas/guide-collection-manifest-v1.schema.json"),
                &serde_json::to_value(document).unwrap(),
            );
        }
        fs::write(&manifest, source).unwrap();
        let output = directory.path().join(format!("collection-{fault}"));
        let build = collection_command("crawlson", "build", &manifest, &output);
        assert_eq!(build.status.code(), Some(4), "{fault}");
        let report: Value = serde_json::from_slice(&build.stdout).unwrap();
        assert_eq!(report["status"], "error");
        assert_eq!(report["diagnostics"][0]["code"], expected_code, "{fault}");
        validate_schema(
            include_str!("../schemas/guide-collection-report-v1.schema.json"),
            &report,
        );
        assert!(!output.exists());
    }
}

#[cfg(unix)]
#[test]
fn guide_collection_rejects_symlinked_inputs_and_outputs() {
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
    let run_report: Value = serde_json::from_slice(&run.stdout).unwrap();
    let root = PathBuf::from(run_report["run_directory"].as_str().unwrap());
    let linked_run = directory.path().join("linked-run");
    symlink(&root, &linked_run).unwrap();
    let manifest = write_collection_manifest(
        directory.path(),
        &[("read-home", 10, &linked_run, &journey)],
    );
    let output = directory.path().join("collection");
    assert_eq!(
        collection_command("crawlson", "build", &manifest, &output)
            .status
            .code(),
        Some(4)
    );

    fs::remove_file(linked_run).unwrap();
    let manifest =
        write_collection_manifest(directory.path(), &[("read-home", 10, &root, &journey)]);
    let real_output = directory.path().join("real-output");
    fs::create_dir(&real_output).unwrap();
    symlink(&real_output, &output).unwrap();
    assert_eq!(
        collection_command("crawlson", "check", &manifest, &output)
            .status
            .code(),
        Some(4)
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

fn collection_command(
    name: &str,
    action: &str,
    manifest: &Path,
    output: &Path,
) -> std::process::Output {
    Command::new(cargo_bin(name))
        .args(["--json", "guides", action])
        .arg(manifest)
        .arg("--output")
        .arg(output)
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap()
}

fn write_collection_manifest(directory: &Path, entries: &[(&str, u32, &Path, &Path)]) -> PathBuf {
    let mut source = r#"schema_version = 1

[collection]
id = "fixture-help"
title = "Fixture Help"
description = "Verified help"

[[topics]]
id = "basics"
title = "Basics"
description = "Complete fixture workflows"
order = 10
audience = ["visitors"]
"#
    .to_owned();
    for (key, order, run, journey) in entries {
        source.push_str(&format!(
            r#"
[[topics.guides]]
key = "{key}"
order = {order}
run = "{}"
journey = "{}"
"#,
            portable_relative(directory, run),
            portable_relative(directory, journey)
        ));
    }
    let path = directory.join("guide-collection.toml");
    fs::write(&path, source).unwrap();
    path
}

fn portable_relative(base: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(base)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| {
            path.canonicalize()
                .unwrap()
                .strip_prefix(base.canonicalize().unwrap())
                .unwrap()
                .to_path_buf()
        });
    relative
        .components()
        .map(|component| component.as_os_str().to_str().unwrap())
        .collect::<Vec<_>>()
        .join("/")
}

fn snapshot_directory(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<(String, Vec<u8>)>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), files);
            } else {
                files.push((
                    portable_relative(root, &entry.path()),
                    fs::read(entry.path()).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    visit(root, root, &mut files);
    files
}

fn write_journey(directory: &Path, expected: &str, authenticated: bool) -> PathBuf {
    let authentication = if authenticated {
        "\n[authentication]\nprovider = \"fixture\"\nrole = \"viewer\"\n"
    } else {
        ""
    };
    let source = format!(
        r##"
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
"##
    );
    let path = directory.join("journey.toml");
    fs::write(&path, source).unwrap();
    path
}

fn write_follow_link_journey(directory: &Path, expected_path: &str) -> PathBuf {
    let source = format!(
        r##"
schema_version = 3

[journey]
id = "fixture.follow-link"
revision = 1
title = "Follow Continue"
purpose = "Verify a visitor can activate the visible Continue link."
expected_outcome = "The completion page is visible."
mode = "read_only"

[target]
origin = "http://127.0.0.1:4173"

[evidence]
trace = true
diagnostics = true

[[steps]]
id = "open"
title = "Open the fixture"
action = {{ type = "navigate", path = "/" }}

[[steps]]
id = "home-heading"
title = "Check the home heading"
action = {{ type = "check_text", selector = "h1", expected = "Hello", comparison = "exact" }}

[[steps]]
id = "follow-continue"
title = "Follow Continue"
guide_instruction = "Select the highlighted Continue link."
action = {{ type = "follow_link", selector = "#action-button", expected_path = "{expected_path}", alt_text = "Continue link highlighted in red" }}

[[steps]]
id = "completion-location"
title = "Check the completion location"
action = {{ type = "check_url", path = "/complete" }}

[[steps]]
id = "completion-heading"
title = "Check the completion heading"
action = {{ type = "check_text", selector = "#completion-heading", expected = "Complete", comparison = "exact" }}
"##
    );
    let path = directory.join("follow-link.toml");
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
