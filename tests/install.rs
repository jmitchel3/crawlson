use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::cargo::cargo_bin;
use crawlson::release::{BUNDLE_MANIFEST_NAME, BundleFileV1, BundleManifestV1, executable_suffix};
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct BundleFixture {
    _directory: TempDir,
    root: PathBuf,
}

impl BundleFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("bundle");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let suffix = executable_suffix(crawlson::BUILD_TARGET).unwrap();
        let mut files = BTreeMap::new();
        for name in ["crawlson", "clson", "crawlson-demo"] {
            let file_name = format!("{name}{suffix}");
            let destination = bin.join(&file_name);
            fs::copy(cargo_bin(name), &destination).unwrap();
            files.insert(format!("bin/{file_name}"), destination);
        }
        let readme = root.join("README.md");
        fs::write(&readme, b"Crawlson installer fixture\n").unwrap();
        files.insert("README.md".to_owned(), readme);
        let manifest = BundleManifestV1 {
            schema_version: 1,
            version: Version::parse(crawlson::VERSION).unwrap(),
            target: crawlson::BUILD_TARGET.to_owned(),
            files: files
                .into_iter()
                .map(|(path, source)| {
                    let bytes = fs::read(source).unwrap();
                    BundleFileV1 {
                        path,
                        size: bytes.len() as u64,
                        sha256: hex_digest(Sha256::digest(bytes)),
                    }
                })
                .collect(),
        };
        write_manifest(&root, &manifest);
        Self {
            _directory: directory,
            root,
        }
    }

    fn manifest(&self) -> BundleManifestV1 {
        serde_json::from_slice(&fs::read(self.root.join(BUNDLE_MANIFEST_NAME)).unwrap()).unwrap()
    }

    fn replace_manifest(&self, update: impl FnOnce(&mut BundleManifestV1)) {
        let mut manifest = self.manifest();
        update(&mut manifest);
        write_manifest(&self.root, &manifest);
    }

    fn executable(&self, name: &str) -> PathBuf {
        let suffix = executable_suffix(crawlson::BUILD_TARGET).unwrap();
        self.root.join("bin").join(format!("{name}{suffix}"))
    }
}

#[test]
fn installs_exact_receipt_and_both_command_names_from_clson() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();

    let output = install_command("clson", &bundle.root, &prefix, state.path(), None);
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "installed");
    assert_eq!(report["version"], crawlson::VERSION);
    assert_eq!(report["target"], crawlson::BUILD_TARGET);

    let installed_crawlson = installed(&prefix, "crawlson");
    let installed_clson = installed(&prefix, "clson");
    assert!(installed_crawlson.is_file());
    assert!(installed_clson.is_file());
    assert!(!installed(&prefix, "crawlson-demo").exists());

    let receipt_bytes = fs::read(state.path().join("install.json")).unwrap();
    let receipt: Value = serde_json::from_slice(&receipt_bytes).unwrap();
    assert_eq!(receipt.as_object().unwrap().len(), 5);
    assert_eq!(receipt["schema_version"], 1);
    assert_eq!(receipt["kind"], "standalone");
    assert_eq!(receipt["target"], crawlson::BUILD_TARGET);
    assert_eq!(
        receipt["binary"],
        serde_json::to_value(fs::canonicalize(&installed_crawlson).unwrap()).unwrap()
    );
    assert!(!receipt["install_id"].as_str().unwrap().is_empty());

    let canonical = version_output(&installed_crawlson, state.path());
    let alias = version_output(&installed_clson, state.path());
    assert_success(&canonical);
    assert_success(&alias);
    assert_eq!(canonical.stdout, alias.stdout);

    let offline = Command::new(&installed_crawlson)
        .args(["--json", "upgrade", "--offline"])
        .env("CRAWLSON_HOME", state.path())
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    assert_eq!(offline.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&offline.stdout).unwrap()["status"],
        "blocked"
    );
}

#[test]
fn managed_reinstall_preserves_install_id_and_repairs_the_alias() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();
    assert_success(&install_command(
        "crawlson",
        &bundle.root,
        &prefix,
        state.path(),
        None,
    ));
    let first_receipt: Value =
        serde_json::from_slice(&fs::read(state.path().join("install.json")).unwrap()).unwrap();
    fs::write(installed(&prefix, "clson"), b"modified alias").unwrap();

    let second = install_command("crawlson", &bundle.root, &prefix, state.path(), None);
    assert_success(&second);
    let second_receipt: Value =
        serde_json::from_slice(&fs::read(state.path().join("install.json")).unwrap()).unwrap();
    assert_eq!(first_receipt["install_id"], second_receipt["install_id"]);
    let alias = version_output(&installed(&prefix, "clson"), state.path());
    assert_success(&alias);
}

#[test]
fn unknown_collision_and_corrupt_receipt_are_blocked_without_overwrite() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();
    fs::create_dir_all(&prefix).unwrap();
    let crawlson = installed(&prefix, "crawlson");
    fs::write(&crawlson, b"not owned").unwrap();

    let collision = install_command("crawlson", &bundle.root, &prefix, state.path(), None);
    assert_blocked(&collision);
    assert_eq!(fs::read(&crawlson).unwrap(), b"not owned");
    assert!(!state.path().join("install.json").exists());

    fs::remove_file(&crawlson).unwrap();
    fs::write(state.path().join("install.json"), b"not json\n").unwrap();
    let corrupt = install_command("crawlson", &bundle.root, &prefix, state.path(), None);
    assert_blocked(&corrupt);
    assert!(!crawlson.exists());
    assert_eq!(
        fs::read(state.path().join("install.json")).unwrap(),
        b"not json\n"
    );
}

#[test]
fn bundle_contract_and_every_digest_are_enforced_before_install() {
    let destination = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);

    let wrong_target = BundleFixture::new();
    wrong_target.replace_manifest(|manifest| manifest.target = "not-a-target".to_owned());
    assert_error(&install_command(
        "crawlson",
        &wrong_target.root,
        &prefix,
        state.path(),
        None,
    ));

    let corrupt = BundleFixture::new();
    fs::write(corrupt.root.join("README.md"), b"tampered\n").unwrap();
    assert_error(&install_command(
        "crawlson",
        &corrupt.root,
        &prefix,
        state.path(),
        None,
    ));

    let missing = BundleFixture::new();
    let demo_path = format!(
        "bin/crawlson-demo{}",
        executable_suffix(crawlson::BUILD_TARGET).unwrap()
    );
    missing.replace_manifest(|manifest| manifest.files.retain(|file| file.path != demo_path));
    fs::remove_file(missing.executable("crawlson-demo")).unwrap();
    assert_error(&install_command(
        "crawlson",
        &missing.root,
        &prefix,
        state.path(),
        None,
    ));

    let broken_demo = BundleFixture::new();
    let demo = broken_demo.executable("crawlson-demo");
    let broken_bytes = b"not an executable demo";
    fs::write(&demo, broken_bytes).unwrap();
    broken_demo.replace_manifest(|manifest| {
        let path = format!(
            "bin/crawlson-demo{}",
            executable_suffix(crawlson::BUILD_TARGET).unwrap()
        );
        let entry = manifest
            .files
            .iter_mut()
            .find(|entry| entry.path == path)
            .unwrap();
        entry.size = broken_bytes.len() as u64;
        entry.sha256 = hex_digest(Sha256::digest(broken_bytes));
    });
    assert_error(&install_command(
        "crawlson",
        &broken_demo.root,
        &prefix,
        state.path(),
        None,
    ));

    assert!(!installed(&prefix, "crawlson").exists());
    assert!(!state.path().join("install.json").exists());
}

#[test]
fn injected_mid_commit_failure_restores_binaries_and_receipt_byte_for_byte() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();
    assert_success(&install_command(
        "crawlson",
        &bundle.root,
        &prefix,
        state.path(),
        None,
    ));
    let crawlson = installed(&prefix, "crawlson");
    let clson = installed(&prefix, "clson");
    let crawlson_before = fs::read(&crawlson).unwrap();
    let receipt_before = fs::read(state.path().join("install.json")).unwrap();
    fs::write(&clson, b"alias bytes that must survive rollback").unwrap();
    let clson_before = fs::read(&clson).unwrap();

    let failed = install_command(
        "crawlson",
        &bundle.root,
        &prefix,
        state.path(),
        Some("after_install_crawlson"),
    );
    assert_error(&failed);
    assert_eq!(fs::read(crawlson).unwrap(), crawlson_before);
    assert_eq!(fs::read(clson).unwrap(), clson_before);
    assert_eq!(
        fs::read(state.path().join("install.json")).unwrap(),
        receipt_before
    );
}

#[test]
fn injected_clean_install_failure_leaves_no_binary_or_receipt() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();
    let failed = install_command(
        "crawlson",
        &bundle.root,
        &prefix,
        state.path(),
        Some("after_install_crawlson"),
    );
    assert_error(&failed);
    assert!(!installed(&prefix, "crawlson").exists());
    assert!(!installed(&prefix, "clson").exists());
    assert!(!state.path().join("install.json").exists());
}

#[test]
fn relative_and_package_manager_prefixes_fail_closed() {
    let bundle = BundleFixture::new();
    let state = tempfile::tempdir().unwrap();
    let relative = install_command(
        "crawlson",
        &bundle.root,
        Path::new("relative/bin"),
        state.path(),
        None,
    );
    assert_blocked(&relative);

    let destination = tempfile::tempdir().unwrap();
    let cargo_bin = fs::canonicalize(destination.path())
        .unwrap()
        .join(".cargo/bin");
    let package_managed = install_command("crawlson", &bundle.root, &cargo_bin, state.path(), None);
    assert_blocked(&package_managed);
    assert!(!installed(&cargo_bin, "crawlson").exists());
}

#[test]
fn unrelated_crawlson_binary_cannot_install_another_bundle() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();
    let output = Command::new(cargo_bin("crawlson"))
        .args(["--json", "install", "--from-bundle"])
        .arg(&bundle.root)
        .arg("--prefix")
        .arg(&prefix)
        .env("CRAWLSON_HOME", state.path())
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CI", "1")
        .output()
        .unwrap();
    assert_blocked(&output);
    assert!(!installed(&prefix, "crawlson").exists());
    assert!(!state.path().join("install.json").exists());
}

#[test]
fn concurrent_update_lock_blocks_install_without_partial_state() {
    let bundle = BundleFixture::new();
    let destination = tempfile::tempdir().unwrap();
    let prefix = test_prefix(&destination);
    let state = tempfile::tempdir().unwrap();
    let lock_path = state.path().join("update.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    lock.try_lock().unwrap();

    let output = install_command("crawlson", &bundle.root, &prefix, state.path(), None);
    assert_blocked(&output);
    assert!(!installed(&prefix, "crawlson").exists());
    assert!(!state.path().join("install.json").exists());
}

#[cfg(unix)]
#[test]
fn symlink_bundle_entries_and_prefix_components_are_rejected() {
    use std::os::unix::fs::symlink;

    let bundle = BundleFixture::new();
    let demo = bundle.executable("crawlson-demo");
    let real_demo = demo.with_extension("real");
    fs::rename(&demo, &real_demo).unwrap();
    symlink(real_demo.file_name().unwrap(), &demo).unwrap();
    let state = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let output = install_command(
        "crawlson",
        &bundle.root,
        &test_prefix(&destination),
        state.path(),
        None,
    );
    assert_error(&output);

    let safe = tempfile::tempdir().unwrap();
    let safe_root = fs::canonicalize(safe.path()).unwrap();
    let real_prefix = safe_root.join("real");
    let linked_prefix = safe_root.join("linked");
    fs::create_dir(&real_prefix).unwrap();
    symlink(&real_prefix, &linked_prefix).unwrap();
    let clean_bundle = BundleFixture::new();
    let output = install_command(
        "crawlson",
        &clean_bundle.root,
        &linked_prefix,
        state.path(),
        None,
    );
    assert_blocked(&output);
}

fn install_command(
    command_name: &str,
    bundle: &Path,
    prefix: &Path,
    home: &Path,
    fault: Option<&str>,
) -> Output {
    let suffix = executable_suffix(crawlson::BUILD_TARGET).unwrap();
    let mut command = Command::new(bundle.join("bin").join(format!("{command_name}{suffix}")));
    command
        .args(["--json", "install", "--from-bundle"])
        .arg(bundle)
        .arg("--prefix")
        .arg(prefix)
        .env("CRAWLSON_HOME", home)
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CI", "1");
    if let Some(fault) = fault {
        command.env("CRAWLSON_TEST_INSTALL_FAIL_AT", fault);
    }
    command.output().unwrap()
}

fn version_output(command: &Path, home: &Path) -> Output {
    Command::new(command)
        .arg("version")
        .env("CRAWLSON_HOME", home)
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CI", "1")
        .output()
        .unwrap()
}

fn installed(prefix: &Path, name: &str) -> PathBuf {
    prefix.join(format!(
        "{name}{}",
        executable_suffix(crawlson::BUILD_TARGET).unwrap()
    ))
}

fn test_prefix(directory: &TempDir) -> PathBuf {
    fs::canonicalize(directory.path()).unwrap().join("bin")
}

fn write_manifest(root: &Path, manifest: &BundleManifestV1) {
    let mut bytes = serde_json::to_vec(manifest).unwrap();
    bytes.push(b'\n');
    fs::write(root.join(BUNDLE_MANIFEST_NAME), bytes).unwrap();
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}

fn assert_blocked(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["status"],
        "blocked"
    );
}

fn assert_error(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["status"],
        "error"
    );
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
