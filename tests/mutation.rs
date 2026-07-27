#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Output};

#[cfg(unix)]
use assert_cmd::cargo::cargo_bin;
use serde_json::Value;
#[cfg(unix)]
use tempfile::TempDir;

#[cfg(unix)]
const LOOPBACK_ORIGIN: &str = "http://127.0.0.1:4173";
#[cfg(unix)]
const REMOTE_ORIGIN: &str = "https://example.test:8443";
#[cfg(unix)]
const MUTATION_GRANTS: [&str; 3] = [
    "demo.mutating-pass@1:fill-fixture-name",
    "demo.mutating-pass@1:create-fixture",
    "demo.mutating-pass@1:ensure-fixture-absent",
];

#[test]
fn published_journey_v5_schema_accepts_the_mutating_example() {
    let document: toml::Value =
        toml::from_str(include_str!("../examples/mutating-pass.toml")).unwrap();
    let document = serde_json::to_value(document).unwrap();

    validate_schema(include_str!("../schemas/journey-v5.schema.json"), &document);
}

#[cfg(unix)]
#[test]
fn runtime_accepts_the_example_and_rejects_an_invalid_v5_contract() {
    let fixture = FakeExecutables::new();
    let directory = tempfile::tempdir().unwrap();
    let journey = example("mutating-pass.toml");

    let accepted = run_journey(
        &journey,
        &directory.path().join("accepted-runs"),
        &fixture,
        None,
        &[],
        &[],
        None,
        false,
    );
    let accepted_report = blocked_report(&accepted, "target_authorization_missing");
    assert_v4_report(&accepted_report);
    fixture.assert_never_launched();

    let invalid = directory.path().join("invalid-v5.toml");
    let source = fs::read_to_string(&journey)
        .unwrap()
        .replace("mode = \"mutating\"", "mode = \"read_only\"");
    fs::write(&invalid, source).unwrap();
    let rejected = run_journey(
        &invalid,
        &directory.path().join("rejected-runs"),
        &fixture,
        None,
        &[],
        &[],
        None,
        false,
    );
    assert_eq!(rejected.status.code(), Some(4));
    let rejected_report: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected_report["outcome"], "error");
    assert_eq!(rejected_report["reason"]["code"], "journey_invalid");
    fixture.assert_never_launched();
}

#[cfg(unix)]
#[test]
fn mutation_grants_are_exact_unique_and_include_cleanup_before_driver_launch() {
    let fixture = FakeExecutables::new();
    let directory = tempfile::tempdir().unwrap();
    let journey = example("mutating-pass.toml");

    let cases: [(&str, Vec<&str>, &str); 4] = [
        ("missing", vec![], "mutation_authorization_mismatch"),
        (
            "extra",
            MUTATION_GRANTS
                .iter()
                .copied()
                .chain(["demo.mutating-pass@1:undeclared-step"])
                .collect(),
            "mutation_authorization_mismatch",
        ),
        (
            "duplicate",
            MUTATION_GRANTS
                .iter()
                .copied()
                .chain([MUTATION_GRANTS[0]])
                .collect(),
            "mutation_authorization_invalid",
        ),
        (
            "cleanup-missing",
            MUTATION_GRANTS[..2].to_vec(),
            "mutation_authorization_mismatch",
        ),
    ];

    for (name, grants, expected_code) in cases {
        let output = run_journey(
            &journey,
            &directory.path().join(format!("{name}-runs")),
            &fixture,
            Some(LOOPBACK_ORIGIN),
            &grants,
            &[],
            None,
            false,
        );
        let report = blocked_report(&output, expected_code);
        assert_v4_report(&report);
        assert_eq!(
            report["mutation_authorization"]["required"],
            serde_json::json!([
                "demo.mutating-pass@1:create-fixture",
                "demo.mutating-pass@1:ensure-fixture-absent",
                "demo.mutating-pass@1:fill-fixture-name"
            ]),
            "{name}"
        );
        fixture.assert_never_launched();
    }
}

#[cfg(unix)]
#[test]
fn mutation_requires_an_explicit_extension_capable_browser_without_leaking_auth() {
    const PATH_SENTINEL: &str = "auth-source-path-sentinel";
    const VALUE_SENTINEL: &str = "auth-storage-value-sentinel";

    let fixture = FakeExecutables::new();
    let directory = tempfile::tempdir().unwrap();
    let auth_state = write_auth_state(
        directory.path(),
        PATH_SENTINEL,
        VALUE_SENTINEL,
        LOOPBACK_ORIGIN,
    );
    let output = run_journey(
        &example("mutating-pass.toml"),
        &directory.path().join("runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        false,
    );
    let report = blocked_report(&output, "extension_browser_missing");
    assert_v4_report(&report);
    assert_eq!(report["authentication"]["status"], "blocked");
    fixture.assert_never_launched();

    let run_root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let persisted_report = fs::read(run_root.join("report.json")).unwrap();
    for sentinel in [PATH_SENTINEL, VALUE_SENTINEL] {
        assert!(!contains_bytes(&output.stdout, sentinel.as_bytes()));
        assert!(!contains_bytes(&output.stderr, sentinel.as_bytes()));
        assert!(!contains_bytes(&persisted_report, sentinel.as_bytes()));
    }
}

#[cfg(unix)]
#[test]
fn non_loopback_mutations_require_the_second_exact_grant_set() {
    let fixture = FakeExecutables::new();
    let directory = tempfile::tempdir().unwrap();
    let journey = rewrite_origin(directory.path(), REMOTE_ORIGIN);

    let missing_second_gate = run_journey(
        &journey,
        &directory.path().join("missing-second-gate-runs"),
        &fixture,
        Some(REMOTE_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        None,
        false,
    );
    let missing_report = blocked_report(
        &missing_second_gate,
        "production_mutation_authorization_mismatch",
    );
    assert_v4_report(&missing_report);
    assert_eq!(
        missing_report["mutation_authorization"]["production_required"],
        true
    );
    assert_eq!(
        missing_report["mutation_authorization"]["production_granted"],
        serde_json::json!([])
    );
    fixture.assert_never_launched();

    let auth_state = write_auth_state(
        directory.path(),
        "remote-auth-state",
        "remote-fixture-session",
        REMOTE_ORIGIN,
    );
    let accepted_second_gate = run_journey(
        &journey,
        &directory.path().join("accepted-second-gate-runs"),
        &fixture,
        Some(REMOTE_ORIGIN),
        &MUTATION_GRANTS,
        &MUTATION_GRANTS,
        Some(&auth_state),
        false,
    );
    let accepted_report = blocked_report(&accepted_second_gate, "extension_browser_missing");
    assert_v4_report(&accepted_report);
    assert_eq!(
        accepted_report["mutation_authorization"]["production_granted"],
        serde_json::json!([
            "demo.mutating-pass@1:create-fixture",
            "demo.mutating-pass@1:ensure-fixture-absent",
            "demo.mutating-pass@1:fill-fixture-name"
        ])
    );
    fixture.assert_never_launched();
}

#[cfg(unix)]
#[test]
fn legacy_journeys_reject_mutation_flags_before_driver_launch() {
    let fixture = FakeExecutables::new();
    let directory = tempfile::tempdir().unwrap();

    for (name, mutation_grants, production_grants) in [
        (
            "mutation",
            vec!["demo.read-home@1:capture-primary-action"],
            vec![],
        ),
        (
            "production",
            vec![],
            vec!["demo.read-home@1:capture-primary-action"],
        ),
    ] {
        let output = run_journey(
            &example("read-only-journey.toml"),
            &directory.path().join(format!("legacy-{name}-runs")),
            &fixture,
            Some(LOOPBACK_ORIGIN),
            &mutation_grants,
            &production_grants,
            None,
            false,
        );
        let report = blocked_report(&output, "mutation_authorization_unexpected");
        assert_eq!(report["schema_version"], 1);
        fixture.assert_never_launched();
    }
}

#[cfg(unix)]
#[test]
fn setup_failure_and_diagnostics_failure_emit_schema_valid_v4_reports() {
    let fixture = FakeExecutables::mutation();
    let directory = tempfile::tempdir().unwrap();
    let journey = example("mutating-pass.toml");
    let auth_state = write_auth_state(
        directory.path(),
        "mutation-auth-state",
        "disposable-session",
        LOOPBACK_ORIGIN,
    );

    fixture.set_scenario("setup_failure");
    let setup = run_journey(
        &journey,
        &directory.path().join("setup-failure-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let setup_report = blocked_report(&setup, "fixture_setup_failed");
    assert_v4_report(&setup_report);
    assert_eq!(setup_report["execution_outcome"], "blocked");
    assert_eq!(setup_report["fixture"]["setup_status"], "blocked");
    assert_eq!(setup_report["fixture"]["mutation_attempted"], false);
    assert_eq!(setup_report["fixture"]["cleanup_status"], "not_needed");
    assert_eq!(setup_report["fixture"]["recovery_required"], false);

    fixture.set_scenario("diagnostics_error");
    let diagnostics = run_journey(
        &journey,
        &directory.path().join("diagnostics-failure-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let diagnostics_report = error_report(&diagnostics, "diagnostics_failed");
    assert_v4_report(&diagnostics_report);
    assert_eq!(diagnostics_report["execution_outcome"], "passed");
    assert!(diagnostics_report.get("diagnostics").is_none());
    assert_eq!(diagnostics_report["fixture"]["cleanup_status"], "passed");
    assert_eq!(diagnostics_report["fixture"]["recovery_required"], false);
}

#[cfg(unix)]
#[test]
fn deterministic_main_mutation_failure_renders_schema_valid_v3_findings_after_cleanup() {
    let fixture = FakeExecutables::mutation();
    fixture.set_scenario("main_failure");
    let directory = tempfile::tempdir().unwrap();
    let journey = example("mutating-pass.toml");
    let auth_state = write_auth_state(
        directory.path(),
        "main-failure-auth-state",
        "disposable-session",
        LOOPBACK_ORIGIN,
    );

    let output = run_journey(
        &journey,
        &directory.path().join("main-failure-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let report = output_report(&output, 1);
    assert_v4_report(&report);
    assert_eq!(report["outcome"], "failed");
    assert_eq!(report["execution_outcome"], "failed");
    assert_eq!(report["reason"]["code"], "checkpoint_failed");
    assert_eq!(report["fixture"]["cleanup_status"], "passed");
    assert_eq!(report["fixture"]["recovery_required"], false);

    let run_root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let rendered = render_journey(&run_root, &journey);
    let render_report = output_report(&rendered, 1);
    assert_eq!(render_report["status"], "findings_ready");
    assert_eq!(render_report["publishable"], true);

    let findings: Value =
        serde_json::from_slice(&fs::read(run_root.join("render/findings.json")).unwrap()).unwrap();
    validate_schema(
        include_str!("../schemas/findings-v3.schema.json"),
        &findings,
    );
    assert_eq!(findings["schema_version"], 3);
    assert_eq!(findings["findings"].as_array().unwrap().len(), 1);
    let finding = &findings["findings"][0];
    assert_eq!(finding["kind"], "mutation_postcondition_mismatch");
    assert_eq!(finding["step"]["id"], "create-fixture");
    assert_eq!(
        finding["reproduction_steps"][0]["action"]["value_source"],
        "generated_public_fixture_token"
    );
}

#[cfg(unix)]
#[test]
fn already_absent_cleanup_renders_and_guard_reuse_is_rejected() {
    let fixture = FakeExecutables::mutation();
    fixture.set_scenario("cleanup_already_absent");
    let directory = tempfile::tempdir().unwrap();
    let journey = example("mutating-pass.toml");
    let auth_state = write_auth_state(
        directory.path(),
        "already-absent-auth-state",
        "disposable-session",
        LOOPBACK_ORIGIN,
    );
    let output = run_journey(
        &journey,
        &directory.path().join("already-absent-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let mut report = success_report(&output);
    assert_v4_report(&report);
    let cleanup = report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "ensure-fixture-absent")
        .unwrap();
    assert_eq!(cleanup["status"], "passed");
    assert_eq!(cleanup["observation"]["action_state"], "effect_verified");
    assert!(
        cleanup["observation"]
            .get("action_command_sequence")
            .is_none()
    );
    assert!(cleanup["observation"].get("artifact_path").is_none());

    let run_root = PathBuf::from(report["run_directory"].as_str().unwrap());
    let rendered = render_journey(&run_root, &journey);
    let render_report = output_report(&rendered, 0);
    assert_eq!(render_report["status"], "guide_ready");
    assert_eq!(render_report["publishable"], true);

    let mut mislabeled_recovery = report.clone();
    mislabeled_recovery["outcome"] = "blocked".into();
    mislabeled_recovery["execution_outcome"] = "blocked".into();
    mislabeled_recovery["reason"] = serde_json::json!({
        "code": "recovery_completed",
        "message": "prior cleanup completed"
    });
    mislabeled_recovery["execution_reason"] = mislabeled_recovery["reason"].clone();
    mislabeled_recovery["fixture"]["setup_status"] = "blocked".into();
    mislabeled_recovery["fixture"]["mutation_attempted"] = false.into();
    mislabeled_recovery["fixture"]["cleanup_status"] = "passed".into();
    mislabeled_recovery["fixture"]["recovery_required"] = false.into();
    assert_v4_report(&mislabeled_recovery);
    fs::write(
        run_root.join("report.json"),
        serde_json::to_vec_pretty(&mislabeled_recovery).unwrap(),
    )
    .unwrap();
    let mislabeled = render_journey(&run_root, &journey);
    let mislabeled_report = output_report(&mislabeled, 4);
    assert_eq!(mislabeled_report["status"], "error");
    assert_eq!(mislabeled_report["reason"]["code"], "report_invalid");

    fs::write(
        run_root.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();

    let guard_sequences = report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["observation"]["guard_command_sequence"].as_u64())
        .collect::<Vec<_>>();
    assert!(guard_sequences.len() >= 2);
    let reused = guard_sequences[0];
    let steps = report["steps"].as_array_mut().unwrap();
    let second_mutation = steps
        .iter_mut()
        .filter(|step| step["effect"] == "mutating")
        .nth(1)
        .unwrap();
    second_mutation["observation"]["guard_command_sequence"] = reused.into();
    fs::write(
        run_root.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert_v4_report(&report);

    let tampered = render_journey(&run_root, &journey);
    let tampered_report = output_report(&tampered, 4);
    assert_eq!(tampered_report["status"], "error");
    assert_eq!(tampered_report["reason"]["code"], "report_invalid");
}

#[cfg(unix)]
#[test]
fn pending_recovery_is_schema_valid_and_exact_rerun_performs_cleanup_only() {
    let fixture = FakeExecutables::mutation();
    let directory = tempfile::tempdir().unwrap();
    let journey = example("mutating-pass.toml");
    let auth_state = write_auth_state(
        directory.path(),
        "recovery-auth-state",
        "disposable-session",
        LOOPBACK_ORIGIN,
    );

    fixture.set_scenario("cleanup_unknown");
    let incomplete = run_journey(
        &journey,
        &directory.path().join("incomplete-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let incomplete_report = error_report(&incomplete, "fixture_cleanup_failed");
    assert_v4_report(&incomplete_report);
    assert_eq!(incomplete_report["execution_outcome"], "passed");
    assert_eq!(
        incomplete_report["fixture"]["cleanup_status"],
        "effect_unknown"
    );
    assert_eq!(incomplete_report["fixture"]["recovery_required"], true);

    fixture.set_scenario("recovery_auth_error");
    let recovery_auth_error = run_journey(
        &journey,
        &directory.path().join("recovery-auth-error-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let recovery_auth_report =
        error_report(&recovery_auth_error, "authentication_state_load_failed");
    assert_v4_report(&recovery_auth_report);
    assert_eq!(recovery_auth_report["fixture"]["mutation_attempted"], false);
    assert_eq!(
        recovery_auth_report["fixture"]["cleanup_status"],
        "not_needed"
    );
    assert_eq!(recovery_auth_report["fixture"]["recovery_required"], true);
    let recovery_auth_root = PathBuf::from(recovery_auth_report["run_directory"].as_str().unwrap());
    let recovery_auth_render = render_journey(&recovery_auth_root, &journey);
    let recovery_auth_render = output_report(&recovery_auth_render, 4);
    assert_eq!(recovery_auth_render["status"], "error");
    assert_eq!(recovery_auth_render["reason"]["code"], "run_error");

    fixture.set_scenario("recovery");
    let recovered = run_journey(
        &journey,
        &directory.path().join("recovery-runs"),
        &fixture,
        Some(LOOPBACK_ORIGIN),
        &MUTATION_GRANTS,
        &[],
        Some(&auth_state),
        true,
    );
    let recovered_report = blocked_report(&recovered, "recovery_completed");
    assert_v4_report(&recovered_report);
    assert_eq!(recovered_report["fixture"]["setup_status"], "blocked");
    assert_eq!(recovered_report["fixture"]["mutation_attempted"], false);
    assert_eq!(recovered_report["fixture"]["cleanup_status"], "passed");
    assert_eq!(recovered_report["fixture"]["recovery_required"], false);
    assert!(
        recovered_report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["phase"] != "journey")
    );

    let recovered_root = PathBuf::from(recovered_report["run_directory"].as_str().unwrap());
    let rendered = render_journey(&recovered_root, &journey);
    let render_report = output_report(&rendered, 3);
    assert_eq!(render_report["status"], "not_publishable");
    assert_eq!(render_report["publishable"], false);
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn run_journey(
    journey: &Path,
    output_directory: &Path,
    fixture: &FakeExecutables,
    allowed_origin: Option<&str>,
    mutation_grants: &[&str],
    production_grants: &[&str],
    auth_state: Option<&Path>,
    include_browser: bool,
) -> Output {
    let mut command = Command::new(cargo_bin("crawlson"));
    command
        .args(["--json", "run"])
        .arg(journey)
        .arg("--output-dir")
        .arg(output_directory)
        .arg("--agent-browser")
        .arg(&fixture.agent_browser)
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CRAWLSON_HOME", &fixture.state_home)
        .env("FAKE_AGENT_BROWSER_LAUNCH_MARKER", &fixture.agent_marker)
        .env("FAKE_CHROMIUM_LAUNCH_MARKER", &fixture.browser_marker);
    if let Some(origin) = allowed_origin {
        command.args(["--allow-origin", origin]);
    }
    for grant in mutation_grants {
        command.args(["--allow-mutation", grant]);
    }
    for grant in production_grants {
        command.args(["--allow-production-mutation", grant]);
    }
    if let Some(path) = auth_state {
        command.arg("--auth-state").arg(path);
    }
    if include_browser {
        command.arg("--browser-executable").arg(&fixture.browser);
    }
    command.output().unwrap()
}

#[cfg(unix)]
fn blocked_report(output: &Output, reason: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "blocked");
    assert_eq!(report["reason"]["code"], reason);
    report
}

#[cfg(unix)]
fn success_report(output: &Output) -> Value {
    let report = output_report(output, 0);
    assert_eq!(report["outcome"], "passed");
    assert_eq!(report["reason"]["code"], "journey_passed");
    report
}

#[cfg(unix)]
fn error_report(output: &Output, reason: &str) -> Value {
    let report = output_report(output, 4);
    assert_eq!(report["outcome"], "error");
    assert_eq!(report["reason"]["code"], reason);
    report
}

#[cfg(unix)]
fn output_report(output: &Output, exit_code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[cfg(unix)]
fn render_journey(run_directory: &Path, journey: &Path) -> Output {
    Command::new(cargo_bin("crawlson"))
        .args(["--json", "render"])
        .arg(run_directory)
        .arg("--journey")
        .arg(journey)
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap()
}

#[cfg(unix)]
fn assert_v4_report(report: &Value) {
    assert_eq!(report["schema_version"], 4);
    validate_schema(include_str!("../schemas/run-report-v4.schema.json"), report);
}

fn validate_schema(source: &str, instance: &Value) {
    let schema: Value = serde_json::from_str(source).unwrap();
    jsonschema::meta::validate(&schema).unwrap();
    if let Err(error) = jsonschema::validator_for(&schema)
        .unwrap()
        .validate(instance)
    {
        panic!("document did not match its published schema: {error}");
    }
}

#[cfg(unix)]
fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

#[cfg(unix)]
fn rewrite_origin(directory: &Path, origin: &str) -> PathBuf {
    let path = directory.join("remote-mutating-pass.toml");
    let source = fs::read_to_string(example("mutating-pass.toml"))
        .unwrap()
        .replace(LOOPBACK_ORIGIN, origin);
    fs::write(&path, source).unwrap();
    path
}

#[cfg(unix)]
fn write_auth_state(directory: &Path, name: &str, value: &str, origin: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(format!("{name}.json"));
    let state = serde_json::json!({
        "cookies": [],
        "origins": [{
            "origin": origin,
            "localStorage": [{"name": "crawlson_demo_session", "value": value}]
        }]
    });
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[cfg(unix)]
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(unix)]
struct FakeExecutables {
    _directory: TempDir,
    agent_browser: PathBuf,
    browser: PathBuf,
    agent_marker: PathBuf,
    browser_marker: PathBuf,
    state_home: PathBuf,
}

#[cfg(unix)]
impl FakeExecutables {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let agent_browser = directory.path().join("agent-browser");
        let browser = directory.path().join("chromium");
        let agent_marker = directory.path().join("agent-browser-launched");
        let browser_marker = directory.path().join("chromium-launched");
        let state_home = directory.path().join("crawlson-state");
        fs::write(
            &agent_browser,
            "#!/bin/sh\ntouch \"$FAKE_AGENT_BROWSER_LAUNCH_MARKER\"\nif [ \"$1\" = \"--version\" ]; then\n  printf 'agent-browser 0.26.0\\n'\n  exit 0\nfi\nexit 90\n",
        )
        .unwrap();
        fs::write(
            &browser,
            "#!/bin/sh\ntouch \"$FAKE_CHROMIUM_LAUNCH_MARKER\"\nexit 91\n",
        )
        .unwrap();
        fs::set_permissions(&agent_browser, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&browser, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _directory: directory,
            agent_browser,
            browser,
            agent_marker,
            browser_marker,
            state_home,
        }
    }

    fn mutation() -> Self {
        let fixture = Self::new();
        fs::write(&fixture.agent_browser, MUTATION_AGENT_BROWSER).unwrap();
        fs::write(
            &fixture.browser,
            "#!/bin/sh\nif [ \"${1-}\" = \"--version\" ]; then\n  printf 'Chromium 138.0.0.0\\n'\n  exit 0\nfi\nexit 91\n",
        )
        .unwrap();
        fixture.enable_screenshot();
        fixture.set_scenario("pass");
        fixture
    }

    fn set_scenario(&self, scenario: &str) {
        fs::write(
            self.agent_browser.parent().unwrap().join("scenario"),
            scenario,
        )
        .unwrap();
    }

    fn enable_screenshot(&self) {
        let file =
            fs::File::create(self.agent_browser.parent().unwrap().join("screenshot.png")).unwrap();
        let mut encoder = png::Encoder::new(file, 1280, 720);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[200, 180, 160, 255].repeat(1280 * 720))
            .unwrap();
        writer.finish().unwrap();
    }

    fn assert_never_launched(&self) {
        assert!(
            !self.agent_marker.exists(),
            "agent-browser launched during a fail-closed preflight"
        );
        assert!(
            !self.browser_marker.exists(),
            "browser launched during a fail-closed preflight"
        );
    }
}

#[cfg(unix)]
const MUTATION_AGENT_BROWSER: &str = r##"#!/bin/sh
set -eu

if [ "${1-}" = "--version" ]; then
  printf 'agent-browser 0.26.0\n'
  exit 0
fi

fixture_dir=${0%/*}
scenario=pass
if [ -f "$fixture_dir/scenario" ]; then
  IFS= read -r scenario < "$fixture_dir/scenario" || true
fi

while [ "$#" -gt 0 ]; do
  if [ "$1" = "--executable-path" ]; then
    shift 2
    break
  fi
  shift
done

success() {
  printf '{"success":true,"data":%s,"error":null}\n' "$1"
}

failure() {
  printf '{"success":false,"data":null,"error":"%s"}\n' "$1"
  exit 1
}

current_url() {
  if [ -f "$fixture_dir/current-url" ]; then
    IFS= read -r url < "$fixture_dir/current-url" || true
    printf '%s' "$url"
  else
    printf 'about:blank'
  fi
}

case "${1-}:${2-}" in
  set:viewport)
    success '{"width":1280,"height":720,"deviceScaleFactor":1.0,"mobile":false}'
    ;;
  state:load)
    if [ "$scenario" = "recovery_auth_error" ]; then
      failure 'authentication state could not be loaded'
    fi
    success "{\"loaded\":true,\"path\":\"$3\"}"
    ;;
  trace:start)
    success '{"started":true}'
    ;;
  trace:stop)
    printf '{"traceEvents":[{"name":"fixture"}]}' > "$3"
    success "{\"path\":\"$3\",\"eventCount\":1}"
    ;;
  open:*)
    printf '%s' "$2" > "$fixture_dir/current-url"
    success "{\"title\":\"Fixture\",\"url\":\"$2\"}"
    ;;
  get:url)
    url=$(current_url)
    success "{\"url\":\"$url\"}"
    ;;
  get:text)
    selector=$3
    case "$selector" in
      '#authenticated-role') text='Viewer access' ;;
      '#fixture-result')
        if [ "$scenario" = "main_failure" ]; then
          text='Disposable fixture was not created.'
        else
          text='Disposable fixture created.'
        fi
        ;;
      '#fixture-empty')
        if [ "$scenario" = "setup_failure" ]; then
          text='Disposable fixture is unexpectedly present.'
        else
          text='Disposable fixture absent.'
        fi
        ;;
      *) text='Fixture text' ;;
    esac
    success "{\"origin\":\"http://127.0.0.1:4173/\",\"text\":\"$text\"}"
    ;;
  is:visible)
    success '{"origin":"http://127.0.0.1:4173/","visible":true}'
    ;;
  is:enabled)
    success '{"origin":"http://127.0.0.1:4173/","enabled":true}'
    ;;
  get:count)
    selector=$3
    case "$selector" in
      *crawlson-exact-origin-guard*) count=1 ;;
      *fixture-empty*)
        if [ -f "$fixture_dir/fixture-present" ]; then count=0; else count=1; fi
        ;;
      *) count=1 ;;
    esac
    success "{\"count\":$count}"
    ;;
  get:value)
    value=''
    if [ -f "$fixture_dir/filled-value" ]; then
      IFS= read -r value < "$fixture_dir/filled-value" || true
    fi
    success "{\"value\":\"$value\"}"
    ;;
  get:attr)
    selector=$3
    attribute=$4
    case "$attribute" in
      method) value='"POST"' ;;
      action)
        case "$selector" in
          *cleanup-form*) value='"/mutation/delete"' ;;
          *) value='"/mutation/create"' ;;
        esac
        ;;
      *) value='null' ;;
    esac
    success "{\"origin\":\"http://127.0.0.1:4173/\",\"value\":$value}"
    ;;
  fill:*)
    printf '%s' "$3" > "$fixture_dir/filled-value"
    success "{\"filled\":\"$2\"}"
    ;;
  click:*)
    selector=$2
    case "$selector" in
      *create-fixture-button*)
        if [ "$scenario" != "cleanup_already_absent" ]; then
          : > "$fixture_dir/fixture-present"
        fi
        printf 'http://127.0.0.1:4173/mutation/created' > "$fixture_dir/current-url"
        ;;
      *remove-fixture-button*)
        if [ "$scenario" = "cleanup_unknown" ]; then
          failure 'cleanup dispatch became uncertain'
        fi
        rm -f "$fixture_dir/fixture-present"
        printf 'http://127.0.0.1:4173/mutation/empty' > "$fixture_dir/current-url"
        ;;
    esac
    success "{\"clicked\":\"$selector\"}"
    ;;
  get:box)
    success '{"x":100.0,"y":100.0,"width":200.0,"height":50.0}'
    ;;
  screenshot:*)
    cp "$fixture_dir/screenshot.png" "$2"
    success "{\"path\":\"$2\"}"
    ;;
  console:)
    if [ "$scenario" = "diagnostics_error" ]; then
      failure 'diagnostics unavailable'
    fi
    success '{"messages":[]}'
    ;;
  errors:)
    success '{"errors":[]}'
    ;;
  close:)
    success '{"closed":true}'
    ;;
  *)
    failure "unexpected fake command: ${1-} ${2-} ${3-}"
    ;;
esac
"##;
