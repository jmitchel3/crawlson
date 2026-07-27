use std::path::PathBuf;
use std::process::Command;

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

    assert!(output.status.success());
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

fn command_output(name: &str, arguments: &[&str]) -> std::process::Output {
    Command::new(cargo_bin(name))
        .args(arguments)
        .output()
        .unwrap()
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
        Self {
            _directory: directory,
            binary,
        }
    }
}
