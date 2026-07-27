use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::release::{
    BUNDLE_MANIFEST_NAME, BundleManifestV1, MAX_RELEASE_FILE_BYTES, executable_suffix,
};
use crate::update::{
    InstallReceipt, UpdatePaths, generated_install_id, package_manager_hint,
    receipt_manages_binary, try_update_lock, write_install_receipt, write_install_receipt_bytes,
};
use crate::{BUILD_TARGET, CommandResult, VERSION};

const MAX_BUNDLE_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub bundle_root: PathBuf,
    pub prefix: PathBuf,
    pub json: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum InstallStatus {
    Installed,
    Blocked,
    Error,
}

#[derive(Debug, Serialize)]
struct InstallReport {
    schema_version: u8,
    status: InstallStatus,
    version: &'static str,
    target: &'static str,
    prefix: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    binary: Option<PathBuf>,
    message: String,
}

#[derive(Debug)]
struct InstallFailure {
    status: InstallStatus,
    message: String,
}

impl InstallFailure {
    fn blocked(message: impl Into<String>) -> Self {
        Self {
            status: InstallStatus::Blocked,
            message: message.into(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            status: InstallStatus::Error,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct VerifiedBundle {
    canonical_root: PathBuf,
    sources: HashMap<&'static str, VerifiedSource>,
}

#[derive(Debug)]
struct VerifiedSource {
    path: PathBuf,
    size: u64,
    sha256: String,
}

#[derive(Debug)]
struct PriorReceipt {
    receipt: InstallReceipt,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct PendingTarget {
    name: &'static str,
    destination: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    had_existing: bool,
    installed: bool,
    backed_up: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct VersionProof {
    schema_version: u8,
    name: String,
    version: String,
    target: String,
}

pub fn run(options: InstallOptions) -> CommandResult {
    let prefix = options.prefix.clone();
    match install(&options) {
        Ok(binary) => render(
            options.json,
            0,
            InstallReport {
                schema_version: 1,
                status: InstallStatus::Installed,
                version: VERSION,
                target: BUILD_TARGET,
                prefix,
                binary: Some(binary),
                message: format!("Crawlson {VERSION}"),
            },
        ),
        Err(failure) => render(
            options.json,
            1,
            InstallReport {
                schema_version: 1,
                status: failure.status,
                version: VERSION,
                target: BUILD_TARGET,
                prefix,
                binary: None,
                message: failure.message,
            },
        ),
    }
}

fn render(json: bool, exit_code: u8, report: InstallReport) -> CommandResult {
    if json {
        let mut stdout = serde_json::to_string(&report).expect("install report is serializable");
        stdout.push('\n');
        CommandResult {
            exit_code,
            stdout,
            stderr: String::new(),
        }
    } else {
        let message = match report.status {
            InstallStatus::Installed => format!("installed: {}\n", report.message),
            InstallStatus::Blocked => format!("installation blocked: {}\n", report.message),
            InstallStatus::Error => format!("installation error: {}\n", report.message),
        };
        if exit_code == 0 {
            CommandResult {
                exit_code,
                stdout: message,
                stderr: String::new(),
            }
        } else {
            CommandResult {
                exit_code,
                stdout: String::new(),
                stderr: message,
            }
        }
    }
}

fn install(options: &InstallOptions) -> Result<PathBuf, InstallFailure> {
    validate_absolute_prefix(&options.prefix)?;
    let bundle = verify_bundle(&options.bundle_root)?;
    let current = env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| InstallFailure::error(format!("could not resolve installer: {error}")))?;
    if current != bundle.sources["crawlson"].path {
        return Err(InstallFailure::blocked(
            "install must be run by bin/crawlson from the selected unpacked bundle",
        ));
    }
    verify_demo(&bundle.sources["demo"].path)?;
    let suffix = executable_suffix(BUILD_TARGET)
        .map_err(|error| InstallFailure::error(error.to_string()))?;
    let crawlson_name = executable_name("crawlson", suffix);
    let clson_name = executable_name("clson", suffix);
    let crawlson_destination = options.prefix.join(&crawlson_name);
    let clson_destination = options.prefix.join(&clson_name);

    for destination in [&crawlson_destination, &clson_destination] {
        if let Some(hint) = package_manager_hint(destination) {
            return Err(InstallFailure::blocked(format!(
                "refusing a first-party install in a package-manager path; use: {hint}"
            )));
        }
    }

    let paths = UpdatePaths::discover().ok_or_else(|| {
        InstallFailure::error("could not determine the Crawlson update-state directory")
    })?;
    if !paths.receipt.is_absolute() || !paths.lock.is_absolute() {
        return Err(InstallFailure::blocked(
            "Crawlson update-state paths must be absolute for a managed install",
        ));
    }
    let _lock = try_update_lock(&paths)
        .map_err(|error| InstallFailure::error(error.to_string()))?
        .ok_or_else(|| InstallFailure::blocked("another Crawlson update is already in progress"))?;

    fs::create_dir_all(&options.prefix).map_err(|error| {
        InstallFailure::error(format!(
            "could not create install prefix {}: {error}",
            options.prefix.display()
        ))
    })?;
    validate_absolute_prefix(&options.prefix)?;
    let canonical_prefix = fs::canonicalize(&options.prefix).map_err(|error| {
        InstallFailure::error(format!(
            "could not resolve install prefix {}: {error}",
            options.prefix.display()
        ))
    })?;
    if canonical_prefix.starts_with(&bundle.canonical_root) {
        return Err(InstallFailure::blocked(
            "the install prefix must be outside the unpacked bundle",
        ));
    }

    let prior = inspect_ownership(&paths, &crawlson_destination, &clson_destination)?;
    let install_id = prior
        .as_ref()
        .map(|prior| prior.receipt.install_id.clone())
        .unwrap_or_else(generated_install_id);

    let transaction = tempfile::Builder::new()
        .prefix(".crawlson-install-")
        .tempdir_in(&options.prefix)
        .map_err(|error| InstallFailure::error(format!("could not stage installation: {error}")))?;

    let mut targets = vec![
        stage_target(
            "crawlson",
            bundle.sources.get("crawlson").expect("verified source"),
            &crawlson_destination,
            transaction.path(),
        )?,
        stage_target(
            "clson",
            bundle.sources.get("clson").expect("verified source"),
            &clson_destination,
            transaction.path(),
        )?,
    ];
    verify_staged_commands(transaction.path(), &crawlson_name, &clson_name)?;

    if let Err(failure) = commit_targets(&mut targets, &paths, prior.as_ref(), &install_id) {
        let rollback = rollback_targets(&mut targets, &paths, prior.as_ref());
        return Err(match rollback {
            Ok(()) => failure,
            Err(rollback_error) => InstallFailure::error(format!(
                "{}; rollback also failed: {}",
                failure.message, rollback_error.message
            )),
        });
    }

    Ok(crawlson_destination)
}

fn verify_staged_commands(
    directory: &Path,
    crawlson_name: &OsString,
    clson_name: &OsString,
) -> Result<(), InstallFailure> {
    let crawlson = version_proof(&directory.join(crawlson_name))?;
    let clson = version_proof(&directory.join(clson_name))?;
    if crawlson != clson {
        return Err(InstallFailure::error(
            "staged clson does not forward the same version report as staged crawlson",
        ));
    }
    if crawlson.schema_version != 1
        || crawlson.name != "crawlson"
        || crawlson.version != VERSION
        || crawlson.target != BUILD_TARGET
    {
        return Err(InstallFailure::error(
            "staged Crawlson version report does not match this installer",
        ));
    }
    Ok(())
}

fn version_proof(binary: &Path) -> Result<VersionProof, InstallFailure> {
    let output = std::process::Command::new(binary)
        .args(["--json", "version"])
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CI", "1")
        .output()
        .map_err(|error| {
            InstallFailure::error(format!(
                "could not run staged {}: {error}",
                binary.display()
            ))
        })?;
    if !output.status.success() || !output.stderr.is_empty() || output.stdout.len() > 16 * 1024 {
        return Err(InstallFailure::error(format!(
            "staged {} did not produce a clean version report",
            binary.display()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        InstallFailure::error(format!(
            "staged {} produced an invalid version report: {error}",
            binary.display()
        ))
    })
}

fn verify_demo(binary: &Path) -> Result<(), InstallFailure> {
    let output = std::process::Command::new(binary)
        .arg("--help")
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CI", "1")
        .output()
        .map_err(|error| {
            InstallFailure::error(format!(
                "could not run bundled demo {}: {error}",
                binary.display()
            ))
        })?;
    if output.status.success() && !output.stdout.is_empty() && output.stdout.len() <= 64 * 1024 {
        Ok(())
    } else {
        Err(InstallFailure::error(
            "bundled crawlson-demo did not produce its expected help output",
        ))
    }
}

fn commit_targets(
    targets: &mut [PendingTarget],
    paths: &UpdatePaths,
    prior: Option<&PriorReceipt>,
    install_id: &str,
) -> Result<(), InstallFailure> {
    for target in &mut *targets {
        if target.had_existing {
            fs::rename(&target.destination, &target.backup).map_err(|error| {
                InstallFailure::error(format!(
                    "could not back up existing {}: {error}",
                    target.destination.display()
                ))
            })?;
            target.backed_up = true;
            test_fault(&format!("after_backup_{}", target.name))?;
        }
    }

    for target in &mut *targets {
        fs::rename(&target.staged, &target.destination).map_err(|error| {
            InstallFailure::error(format!(
                "could not install {}: {error}",
                target.destination.display()
            ))
        })?;
        target.installed = true;
        test_fault(&format!("after_install_{}", target.name))?;
    }

    test_fault("before_receipt")?;
    let receipt = InstallReceipt {
        schema_version: 1,
        kind: "standalone".to_owned(),
        target: BUILD_TARGET.to_owned(),
        binary: fs::canonicalize(&targets[0].destination).map_err(|error| {
            InstallFailure::error(format!(
                "could not resolve installed Crawlson binary: {error}"
            ))
        })?,
        install_id: install_id.to_owned(),
    };
    if let Err(error) = write_install_receipt(&paths.receipt, &receipt) {
        restore_receipt(paths, prior)?;
        return Err(InstallFailure::error(format!(
            "could not commit managed-install receipt: {error}"
        )));
    }
    Ok(())
}

fn rollback_targets(
    targets: &mut [PendingTarget],
    paths: &UpdatePaths,
    prior: Option<&PriorReceipt>,
) -> Result<(), InstallFailure> {
    let mut errors = Vec::new();
    for target in targets.iter_mut().rev() {
        if target.installed {
            if let Err(error) = fs::remove_file(&target.destination)
                && error.kind() != io::ErrorKind::NotFound
            {
                errors.push(format!(
                    "could not remove {}: {error}",
                    target.destination.display()
                ));
            }
            target.installed = false;
        }
        if target.backed_up {
            if let Err(error) = fs::rename(&target.backup, &target.destination) {
                errors.push(format!(
                    "could not restore {}: {error}",
                    target.destination.display()
                ));
            } else {
                target.backed_up = false;
            }
        }
    }
    if let Err(error) = restore_receipt(paths, prior) {
        errors.push(error.message);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(InstallFailure::error(errors.join("; ")))
    }
}

fn restore_receipt(
    paths: &UpdatePaths,
    prior: Option<&PriorReceipt>,
) -> Result<(), InstallFailure> {
    match prior {
        Some(prior) => write_install_receipt_bytes(&paths.receipt, &prior.bytes)
            .map_err(|error| InstallFailure::error(error.to_string())),
        None => match fs::remove_file(&paths.receipt) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(InstallFailure::error(format!(
                "could not remove incomplete receipt: {error}"
            ))),
        },
    }
}

fn inspect_ownership(
    paths: &UpdatePaths,
    crawlson: &Path,
    clson: &Path,
) -> Result<Option<PriorReceipt>, InstallFailure> {
    for target in [crawlson, clson] {
        if let Ok(metadata) = fs::symlink_metadata(target)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(InstallFailure::blocked(format!(
                "refusing to replace non-regular install target {}",
                target.display()
            )));
        }
    }

    let crawlson_exists = crawlson.exists();
    let clson_exists = clson.exists();
    let receipt_metadata = match fs::symlink_metadata(&paths.receipt) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(InstallFailure::error(format!(
                "could not inspect managed-install receipt: {error}"
            )));
        }
    };
    if receipt_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(InstallFailure::blocked(
            "managed-install receipt is not a regular file",
        ));
    }

    let prior = if receipt_metadata.is_some() {
        let bytes = fs::read(&paths.receipt).map_err(|error| {
            InstallFailure::error(format!("could not read managed-install receipt: {error}"))
        })?;
        let receipt: InstallReceipt = serde_json::from_slice(&bytes).map_err(|error| {
            InstallFailure::blocked(format!("managed-install receipt is invalid: {error}"))
        })?;
        if !crawlson_exists || !receipt_manages_binary(&receipt, crawlson) {
            return Err(InstallFailure::blocked(
                "an existing managed-install receipt does not own this exact destination",
            ));
        }
        Some(PriorReceipt { receipt, bytes })
    } else {
        None
    };

    if (crawlson_exists || clson_exists) && prior.is_none() {
        return Err(InstallFailure::blocked(
            "install targets already exist without an exact Crawlson managed-install receipt",
        ));
    }
    Ok(prior)
}

fn stage_target(
    name: &'static str,
    source: &VerifiedSource,
    destination: &Path,
    transaction: &Path,
) -> Result<PendingTarget, InstallFailure> {
    let file_name = destination
        .file_name()
        .ok_or_else(|| InstallFailure::error("install target has no file name"))?;
    let staged = transaction.join(file_name);
    fs::copy(&source.path, &staged).map_err(|error| {
        InstallFailure::error(format!(
            "could not stage {} from {}: {error}",
            destination.display(),
            source.path.display()
        ))
    })?;
    let staged_metadata = regular_file_metadata(&staged)?;
    if staged_metadata.len() != source.size || sha256_file(&staged, source.size)? != source.sha256 {
        return Err(InstallFailure::error(format!(
            "staged {} changed after bundle verification",
            source.path.display()
        )));
    }
    make_executable(&staged)?;
    File::open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|error| InstallFailure::error(format!("could not sync staged binary: {error}")))?;
    let backup = transaction.join(format!("backup-{}", file_name.to_string_lossy()));
    Ok(PendingTarget {
        name,
        destination: destination.to_owned(),
        staged,
        backup,
        had_existing: destination.exists(),
        installed: false,
        backed_up: false,
    })
}

fn verify_bundle(root: &Path) -> Result<VerifiedBundle, InstallFailure> {
    let root_metadata = fs::symlink_metadata(root).map_err(|error| {
        InstallFailure::error(format!(
            "could not inspect bundle root {}: {error}",
            root.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(InstallFailure::error(
            "bundle root must be a regular directory, not a symlink",
        ));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        InstallFailure::error(format!(
            "could not resolve bundle root {}: {error}",
            root.display()
        ))
    })?;
    let manifest_path = canonical_root.join(BUNDLE_MANIFEST_NAME);
    let metadata = regular_file_metadata(&manifest_path)?;
    if metadata.len() == 0 || metadata.len() > MAX_BUNDLE_MANIFEST_BYTES {
        return Err(InstallFailure::error(
            "bundle manifest size is outside the accepted range",
        ));
    }
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        InstallFailure::error(format!("could not read bundle manifest: {error}"))
    })?;
    let manifest: BundleManifestV1 = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| InstallFailure::error(format!("bundle manifest is invalid: {error}")))?;
    manifest
        .validate()
        .map_err(|error| InstallFailure::error(format!("bundle manifest is invalid: {error}")))?;
    let version = Version::parse(VERSION).expect("package version is valid SemVer");
    if manifest.version != version {
        return Err(InstallFailure::error(format!(
            "bundle version {} does not match installer version {VERSION}",
            manifest.version
        )));
    }
    if manifest.target != BUILD_TARGET {
        return Err(InstallFailure::error(format!(
            "bundle target {} does not match installer target {BUILD_TARGET}",
            manifest.target
        )));
    }

    let suffix = executable_suffix(BUILD_TARGET)
        .map_err(|error| InstallFailure::error(error.to_string()))?;
    let required = [
        ("crawlson", format!("bin/crawlson{suffix}")),
        ("clson", format!("bin/clson{suffix}")),
        ("demo", format!("bin/crawlson-demo{suffix}")),
    ];
    let expected: HashSet<String> = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    for (_, path) in &required {
        if !expected.contains(path) {
            return Err(InstallFailure::error(format!(
                "bundle manifest is missing required binary {path}"
            )));
        }
    }
    if expected.contains(BUNDLE_MANIFEST_NAME) {
        return Err(InstallFailure::error(
            "bundle manifest must not contain a digest for itself",
        ));
    }

    let actual = collect_bundle_files(&canonical_root)?;
    let mut allowed = expected.clone();
    allowed.insert(BUNDLE_MANIFEST_NAME.to_owned());
    if actual != allowed {
        let unexpected = actual.difference(&allowed).next();
        let missing = allowed.difference(&actual).next();
        return Err(InstallFailure::error(match (unexpected, missing) {
            (Some(path), _) => format!("bundle contains undeclared file {path}"),
            (_, Some(path)) => format!("bundle is missing declared file {path}"),
            _ => "bundle contents do not match the manifest".to_owned(),
        }));
    }

    for entry in &manifest.files {
        let path = canonical_root.join(&entry.path);
        let metadata = regular_file_metadata(&path)?;
        if metadata.len() != entry.size || metadata.len() > MAX_RELEASE_FILE_BYTES {
            return Err(InstallFailure::error(format!(
                "bundle file {} has the wrong size",
                entry.path
            )));
        }
        let digest = sha256_file(&path, entry.size)?;
        if digest != entry.sha256 {
            return Err(InstallFailure::error(format!(
                "bundle file {} failed SHA-256 verification",
                entry.path
            )));
        }
    }

    let sources = required
        .into_iter()
        .map(|(name, path)| {
            let entry = manifest
                .files
                .iter()
                .find(|entry| entry.path == path)
                .expect("required entry was checked");
            (
                name,
                VerifiedSource {
                    path: canonical_root.join(path),
                    size: entry.size,
                    sha256: entry.sha256.clone(),
                },
            )
        })
        .collect();
    Ok(VerifiedBundle {
        canonical_root,
        sources,
    })
}

fn collect_bundle_files(root: &Path) -> Result<HashSet<String>, InstallFailure> {
    let mut files = Vec::new();
    collect_bundle_files_at(root, root, &mut files)?;
    Ok(files.into_iter().collect())
}

fn collect_bundle_files_at(
    root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
) -> Result<(), InstallFailure> {
    let entries = fs::read_dir(directory).map_err(|error| {
        InstallFailure::error(format!(
            "could not inspect bundle directory {}: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| InstallFailure::error(error.to_string()))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| InstallFailure::error(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(InstallFailure::error(format!(
                "bundle entry {} is a symlink",
                path.display()
            )));
        }
        if metadata.is_dir() {
            collect_bundle_files_at(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("walked path remains under bundle root");
            files.push(portable_relative_path(relative)?);
        } else {
            return Err(InstallFailure::error(format!(
                "bundle entry {} is not a regular file",
                path.display()
            )));
        }
    }
    Ok(())
}

fn portable_relative_path(path: &Path) -> Result<String, InstallFailure> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(InstallFailure::error("bundle contains an unsafe path"));
        };
        let part = part
            .to_str()
            .ok_or_else(|| InstallFailure::error("bundle contains a non-UTF-8 path"))?;
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn regular_file_metadata(path: &Path) -> Result<fs::Metadata, InstallFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        InstallFailure::error(format!(
            "could not inspect bundle file {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(InstallFailure::error(format!(
            "bundle entry {} is not a regular non-symlink file",
            path.display()
        )));
    }
    Ok(metadata)
}

fn sha256_file(path: &Path, expected_size: u64) -> Result<String, InstallFailure> {
    let mut file = File::open(path).map_err(|error| InstallFailure::error(error.to_string()))?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| InstallFailure::error(error.to_string()))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > expected_size || total > MAX_RELEASE_FILE_BYTES {
            return Err(InstallFailure::error(format!(
                "bundle file {} exceeded its declared size",
                path.display()
            )));
        }
        digest.update(&buffer[..read]);
    }
    if total != expected_size {
        return Err(InstallFailure::error(format!(
            "bundle file {} changed during verification",
            path.display()
        )));
    }
    Ok(hex_digest(digest.finalize()))
}

fn validate_absolute_prefix(prefix: &Path) -> Result<(), InstallFailure> {
    if !prefix.is_absolute() {
        return Err(InstallFailure::blocked(
            "--prefix must be an explicit absolute directory",
        ));
    }
    let mut current = PathBuf::new();
    for component in prefix.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                current.push(component.as_os_str());
            }
            Component::CurDir | Component::ParentDir => {
                return Err(InstallFailure::blocked(
                    "--prefix must be a normalized absolute directory",
                ));
            }
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(InstallFailure::blocked(format!(
                    "install prefix contains symlink component {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(InstallFailure::blocked(format!(
                    "install prefix component {} is not a directory",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(InstallFailure::error(format!(
                    "could not inspect install prefix {}: {error}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

fn executable_name(stem: &str, suffix: &str) -> OsString {
    let mut name = OsString::from(stem);
    name.push(suffix);
    name
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), InstallFailure> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| InstallFailure::error(error.to_string()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| InstallFailure::error(error.to_string()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), InstallFailure> {
    Ok(())
}

fn test_fault(point: &str) -> Result<(), InstallFailure> {
    if cfg!(debug_assertions)
        && env::var("CRAWLSON_TEST_INSTALL_FAIL_AT").is_ok_and(|value| value == point)
    {
        Err(InstallFailure::error(format!(
            "injected installer failure at {point}"
        )))
    } else {
        Ok(())
    }
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
