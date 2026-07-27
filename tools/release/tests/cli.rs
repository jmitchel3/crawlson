use assert_cmd::Command;

#[test]
fn help_exposes_only_the_dry_run_release_operations() {
    let output = Command::cargo_bin("crawlson-release")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in ["package", "assemble", "generate-test-key", "sign"] {
        assert!(stdout.contains(command), "missing {command}: {stdout}");
    }
    assert!(!stdout.contains("publish"));
    assert!(!stdout.contains("skip-native-smoke"));
}

#[test]
fn generate_test_key_cli_writes_only_the_named_pair_and_refuses_a_second_write() {
    let root = tempfile::tempdir().unwrap();
    let first = Command::cargo_bin("crawlson-release")
        .unwrap()
        .args(["generate-test-key", "--out-dir"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let stdout = String::from_utf8(first.stdout).unwrap();
    assert!(stdout.contains("crawlson-dry-run-test-only.pub"));
    assert!(stdout.contains("crawlson-dry-run-test-only.key"));

    let second = Command::cargo_bin("crawlson-release")
        .unwrap()
        .args(["generate-test-key", "--out-dir"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(
        String::from_utf8(second.stderr)
            .unwrap()
            .contains("refusing to overwrite")
    );
}
