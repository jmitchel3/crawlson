use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crawlson::release::{
    BUNDLE_MANIFEST_NAME, BundleFileV1, BundleFormat, BundleManifestV1, MAX_RELEASE_FILE_BYTES,
    RELEASE_INVENTORY_NAME, RELEASE_SIGNATURE_NAME, ReleaseBundleV1, ReleaseContractError,
    ReleaseInventoryV1, SUPPORTED_RELEASE_TARGETS, UPDATE_MANIFEST_NAME, UPDATE_SIGNATURE_NAME,
    UpdateArtifactV1, UpdateManifestV1, bundle_artifact_name, bundle_format, bundle_root_name,
    executable_suffix, update_artifact_name,
};
use flate2::Compression;
use flate2::GzBuilder;
use flate2::read::GzDecoder;
use minisign::{KeyPair, PublicKeyBox, SecretKeyBox};
use minisign_verify::{PublicKey as VerifyPublicKey, Signature as VerifySignature};
use semver::Version;
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const TEST_SECRET_KEY_NAME: &str = "crawlson-dry-run-test-only.key";
pub const TEST_PUBLIC_KEY_NAME: &str = "crawlson-dry-run-test-only.pub";
pub const FRAGMENT_SUFFIX: &str = ".fragment.json";

const TEST_KEY_PASSWORD: &str = "crawlson-dry-run-test-only";
const MAX_CONTRACT_BYTES: u64 = 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 32;
const MAX_ARCHIVE_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ReleaseToolError {
    #[error(transparent)]
    Contract(#[from] ReleaseContractError),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path}: invalid JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("refusing to overwrite existing output {0}")]
    OutputExists(PathBuf),
    #[error("release input is not a regular non-symlink file: {0}")]
    NotRegular(PathBuf),
    #[error("release input exceeds its size limit: {0}")]
    TooLarge(PathBuf),
    #[error("release input is empty: {0}")]
    Empty(PathBuf),
    #[error("release package is invalid: {0}")]
    InvalidPackage(String),
    #[error("release matrix is incomplete; expected exactly {expected}, found {actual}")]
    IncompleteMatrix { expected: String, actual: String },
    #[error("release signing failed: {0}")]
    Signing(String),
    #[error("release signature verification failed: {0}")]
    Verification(String),
}

#[derive(Debug, Clone)]
pub struct PackageOptions {
    pub target: String,
    pub bin_dir: PathBuf,
    pub source_dir: PathBuf,
    pub out_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageOutputs {
    pub bundle: PathBuf,
    pub update: PathBuf,
    pub fragment: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AssembleOptions {
    pub inputs: Vec<PathBuf>,
    pub out_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembleOutputs {
    pub update_manifest: PathBuf,
    pub release_inventory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestKeyOutputs {
    pub secret_key: PathBuf,
    pub public_key: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SignOptions {
    pub secret_key: PathBuf,
    pub public_key: Option<PathBuf>,
    pub inputs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct ArchiveFile {
    bytes: Vec<u8>,
    mode: u32,
}

pub fn package(options: &PackageOptions) -> Result<PackageOutputs, ReleaseToolError> {
    package_impl(options, true)
}

fn package_impl(
    options: &PackageOptions,
    require_native_smoke: bool,
) -> Result<PackageOutputs, ReleaseToolError> {
    let version = current_version()?;
    let format = bundle_format(&options.target)?;
    let suffix = executable_suffix(&options.target)?;
    validate_directory(&options.bin_dir)?;
    validate_directory(&options.source_dir)?;
    prepare_output_directory(&options.out_dir)?;

    if require_native_smoke {
        validate_native_binaries(options, &version, suffix)?;
    }

    let mut files = BTreeMap::new();
    for name in ["crawlson", "clson", "crawlson-demo"] {
        let file_name = format!("{name}{suffix}");
        let source = options.bin_dir.join(&file_name);
        insert_source(&mut files, format!("bin/{file_name}"), &source, 0o755)?;
    }
    for name in ["demo-fail.toml", "demo-pass.toml"] {
        let source = options.source_dir.join("examples").join(name);
        insert_source(&mut files, format!("examples/{name}"), &source, 0o644)?;
    }
    let source = options.source_dir.join("scripts/demo.sh");
    insert_source(&mut files, "scripts/demo.sh".to_owned(), &source, 0o755)?;
    files.insert(
        "README.md".to_owned(),
        ArchiveFile {
            bytes: generated_readme(&version, &options.target),
            mode: 0o644,
        },
    );
    let unpacked_size = files.values().try_fold(0_u64, |total, file| {
        total.checked_add(file.bytes.len() as u64)
    });
    if unpacked_size.is_none_or(|size| size > MAX_ARCHIVE_UNPACKED_BYTES) {
        return Err(ReleaseToolError::InvalidPackage(
            "bundle payload exceeds the total unpacked size limit".to_owned(),
        ));
    }

    let file_contracts = files
        .iter()
        .map(|(path, file)| BundleFileV1 {
            path: path.clone(),
            size: file.bytes.len() as u64,
            sha256: digest(&file.bytes),
        })
        .collect::<Vec<_>>();
    let manifest = BundleManifestV1 {
        schema_version: 1,
        version: version.clone(),
        target: options.target.clone(),
        files: file_contracts.clone(),
    };
    manifest.validate()?;
    files.insert(
        BUNDLE_MANIFEST_NAME.to_owned(),
        ArchiveFile {
            bytes: canonical_json(&manifest)?,
            mode: 0o644,
        },
    );

    let root = bundle_root_name(&version, &options.target);
    let bundle_bytes = match format {
        BundleFormat::TarGz => build_tar_gz(&root, &files)?,
        BundleFormat::Zip => build_zip(&root, &files)?,
    };
    enforce_generated_size("bundle", &bundle_bytes)?;
    let crawlson_path = format!("bin/crawlson{suffix}");
    let update_bytes = files
        .get(&crawlson_path)
        .expect("canonical crawlson payload was inserted")
        .bytes
        .clone();

    let bundle_name = bundle_artifact_name(&version, &options.target, format);
    let update_name = update_artifact_name(&version, &options.target);
    let bundle = ReleaseBundleV1 {
        target: options.target.clone(),
        format,
        name: bundle_name.clone(),
        size: bundle_bytes.len() as u64,
        sha256: digest(&bundle_bytes),
        update_name: update_name.clone(),
        update_size: update_bytes.len() as u64,
        update_sha256: digest(&update_bytes),
        files: file_contracts,
    };
    let fragment_contract = ReleaseInventoryV1 {
        schema_version: 1,
        version,
        bundles: vec![bundle],
    };
    fragment_contract.validate()?;
    let fragment_name = format!("{root}{FRAGMENT_SUFFIX}");

    let bundle_path = options.out_dir.join(bundle_name);
    let update_path = options.out_dir.join(update_name);
    let fragment_path = options.out_dir.join(fragment_name);
    publish_files(&[
        (bundle_path.clone(), bundle_bytes),
        (update_path.clone(), update_bytes),
        (fragment_path.clone(), canonical_json(&fragment_contract)?),
    ])?;

    Ok(PackageOutputs {
        bundle: bundle_path,
        update: update_path,
        fragment: fragment_path,
    })
}

pub fn assemble(options: &AssembleOptions) -> Result<AssembleOutputs, ReleaseToolError> {
    if options.inputs.is_empty() {
        return Err(ReleaseToolError::IncompleteMatrix {
            expected: expected_targets(),
            actual: "none".to_owned(),
        });
    }
    prepare_output_directory(&options.out_dir)?;
    let mut by_target = BTreeMap::<String, ReleaseBundleV1>::new();
    let mut version: Option<Version> = None;

    for path in &options.inputs {
        let bytes = read_regular(path, MAX_CONTRACT_BYTES)?;
        let fragment: ReleaseInventoryV1 = parse_json(path, &bytes)?;
        fragment.validate()?;
        if fragment.bundles.len() != 1 {
            return Err(ReleaseToolError::InvalidPackage(format!(
                "{} must contain exactly one bundle",
                path.display()
            )));
        }
        if let Some(expected) = &version {
            if expected != &fragment.version {
                return Err(ReleaseToolError::InvalidPackage(format!(
                    "{} has version {}, expected {expected}",
                    path.display(),
                    fragment.version
                )));
            }
        } else {
            version = Some(fragment.version.clone());
        }

        let bundle = fragment.bundles.into_iter().next().unwrap();
        if by_target.contains_key(&bundle.target) {
            return Err(ReleaseContractError::Duplicate(bundle.target).into());
        }
        verify_fragment_files(path, &fragment.version, &bundle)?;
        by_target.insert(bundle.target.clone(), bundle);
    }

    require_complete_matrix(by_target.keys().map(String::as_str))?;
    let version = version.expect("nonempty fragments establish a version");
    let bundles = by_target.into_values().collect::<Vec<_>>();
    let artifacts = bundles
        .iter()
        .map(|bundle| UpdateArtifactV1 {
            target: bundle.target.clone(),
            name: bundle.update_name.clone(),
            size: bundle.update_size,
            sha256: bundle.update_sha256.clone(),
        })
        .collect::<Vec<_>>();
    let update = UpdateManifestV1 {
        schema_version: 1,
        version: version.clone(),
        artifacts,
    };
    let inventory = ReleaseInventoryV1 {
        schema_version: 1,
        version,
        bundles,
    };
    update.validate()?;
    inventory.validate()?;
    require_complete_matrix(update.artifacts.iter().map(|item| item.target.as_str()))?;
    require_complete_matrix(inventory.bundles.iter().map(|item| item.target.as_str()))?;

    let update_path = options.out_dir.join(UPDATE_MANIFEST_NAME);
    let inventory_path = options.out_dir.join(RELEASE_INVENTORY_NAME);
    publish_files(&[
        (update_path.clone(), canonical_json(&update)?),
        (inventory_path.clone(), canonical_json(&inventory)?),
    ])?;
    Ok(AssembleOutputs {
        update_manifest: update_path,
        release_inventory: inventory_path,
    })
}

pub fn generate_test_key(out_dir: &Path) -> Result<TestKeyOutputs, ReleaseToolError> {
    prepare_output_directory(out_dir)?;
    let secret_key = out_dir.join(TEST_SECRET_KEY_NAME);
    let public_key = out_dir.join(TEST_PUBLIC_KEY_NAME);
    ensure_outputs_absent([secret_key.as_path(), public_key.as_path()])?;

    let pair = KeyPair::generate_encrypted_keypair(Some(TEST_KEY_PASSWORD.to_owned()))
        .map_err(signing_error)?;
    let public_bytes = with_one_lf(pair.pk.to_box().map_err(signing_error)?.to_bytes());
    let secret_bytes = with_one_lf(
        pair.sk
            .to_box(Some("Crawlson dry-run test-only secret key"))
            .map_err(signing_error)?
            .to_bytes(),
    );
    publish_test_key_pair(&public_key, &public_bytes, &secret_key, &secret_bytes)?;
    Ok(TestKeyOutputs {
        secret_key,
        public_key,
    })
}

pub fn sign(options: &SignOptions) -> Result<Vec<PathBuf>, ReleaseToolError> {
    if options.inputs.is_empty() {
        return Err(ReleaseToolError::Signing(
            "at least one canonical manifest is required".to_owned(),
        ));
    }
    let public_path = options
        .public_key
        .clone()
        .unwrap_or_else(|| sibling_public_key(&options.secret_key));
    let secret_text = read_utf8_regular(&options.secret_key, MAX_CONTRACT_BYTES)?;
    let public_text = read_utf8_regular(&public_path, MAX_CONTRACT_BYTES)?;
    let secret = SecretKeyBox::from_string(&secret_text)
        .map_err(signing_error)?
        .into_secret_key(Some(TEST_KEY_PASSWORD.to_owned()))
        .map_err(signing_error)?;
    let public = PublicKeyBox::from_string(&public_text)
        .map_err(signing_error)?
        .into_public_key()
        .map_err(signing_error)?;
    let derived = minisign::PublicKey::from_secret_key(&secret).map_err(signing_error)?;
    if derived.to_bytes() != public.to_bytes() {
        return Err(ReleaseToolError::Signing(
            "secret and public keys are not a pair".to_owned(),
        ));
    }

    let mut pending = Vec::new();
    let mut seen = BTreeSet::new();
    for input in &options.inputs {
        let bytes = read_regular(input, MAX_CONTRACT_BYTES)?;
        validate_signable(input, &bytes)?;
        let output = signature_path(input)?;
        if !seen.insert(output.clone()) {
            return Err(ReleaseContractError::Duplicate(output.display().to_string()).into());
        }
        if output.exists() {
            return Err(ReleaseToolError::OutputExists(output));
        }
        let signature = minisign::sign(
            Some(&public),
            &secret,
            Cursor::new(&bytes),
            Some("crawlson dry-run test-only signature"),
            Some("signature from Crawlson dry-run test-only key"),
        )
        .map_err(signing_error)?;
        let signature_bytes = with_one_lf(signature.into_string().into_bytes());
        verify_signature_bytes(&public_text, &bytes, &signature_bytes)?;
        pending.push((output, signature_bytes));
    }
    let outputs = pending.iter().map(|(path, _)| path.clone()).collect();
    publish_files(&pending)?;
    Ok(outputs)
}

pub fn verify_signature(
    public_key: &Path,
    input: &Path,
    signature: &Path,
) -> Result<(), ReleaseToolError> {
    let public_text = read_utf8_regular(public_key, MAX_CONTRACT_BYTES)?;
    let input_bytes = read_regular(input, MAX_CONTRACT_BYTES)?;
    let signature_bytes = read_regular(signature, MAX_CONTRACT_BYTES)?;
    verify_signature_bytes(&public_text, &input_bytes, &signature_bytes)
}

fn validate_native_binaries(
    options: &PackageOptions,
    version: &Version,
    suffix: &str,
) -> Result<(), ReleaseToolError> {
    let crawlson = options.bin_dir.join(format!("crawlson{suffix}"));
    let clson = options.bin_dir.join(format!("clson{suffix}"));
    let demo = options.bin_dir.join(format!("crawlson-demo{suffix}"));
    for path in [&crawlson, &clson, &demo] {
        let _ = open_regular(path, MAX_RELEASE_FILE_BYTES)?;
    }

    let canonical = command_output(&crawlson, &["--json", "version"])?;
    let alias = command_output(&clson, &["--json", "version"])?;
    if canonical != alias {
        return Err(ReleaseToolError::InvalidPackage(
            "crawlson and clson version reports differ".to_owned(),
        ));
    }
    let report: serde_json::Value =
        serde_json::from_slice(&canonical).map_err(|source| ReleaseToolError::Json {
            path: crawlson.clone(),
            source,
        })?;
    let expected_version = version.to_string();
    if report
        .get("schema_version")
        .and_then(|value| value.as_u64())
        != Some(1)
        || report.get("name").and_then(|value| value.as_str()) != Some("crawlson")
        || report.get("version").and_then(|value| value.as_str()) != Some(expected_version.as_str())
        || report.get("target").and_then(|value| value.as_str()) != Some(options.target.as_str())
    {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "{} did not report version {version} for target {}",
            crawlson.display(),
            options.target
        )));
    }

    let demo_version = command_output(&demo, &["--version"])?;
    let expected_demo_version = format!("crawlson-demo {version}\n");
    if demo_version != expected_demo_version.as_bytes() {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "{} did not report version {version}",
            demo.display()
        )));
    }
    smoke_demo_startup(&demo)
}

fn command_output(path: &Path, args: &[&str]) -> Result<Vec<u8>, ReleaseToolError> {
    let output = ProcessCommand::new(path)
        .args(args)
        .env("CI", "true")
        .env("CRAWLSON_NO_UPDATE_CHECK", "1")
        .env("CRAWLSON_OFFLINE", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| io_error(path.to_owned(), source))?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "{} failed its native smoke check: status {}, stderr {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

fn smoke_demo_startup(path: &Path) -> Result<(), ReleaseToolError> {
    let mut child = ProcessCommand::new(path)
        .args(["--port", "0", "--json"])
        .env("CI", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| io_error(path.to_owned(), source))?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ReleaseToolError::InvalidPackage("demo stdout was not captured".to_owned())
    })?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || sender.send(read_line_bounded(stdout)).ok());
    let readiness = receiver.recv_timeout(Duration::from_secs(5));
    let _ = child.kill();
    let wait = child
        .wait()
        .map_err(|source| io_error(path.to_owned(), source));
    let _ = reader.join();
    wait?;
    let bytes = readiness
        .map_err(|_| {
            ReleaseToolError::InvalidPackage(
                "demo did not become ready within 5 seconds".to_owned(),
            )
        })?
        .map_err(|source| io_error(path.to_owned(), source))?;
    let report: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| ReleaseToolError::Json {
            path: path.to_owned(),
            source,
        })?;
    let origin = report.get("origin").and_then(|value| value.as_str());
    if report
        .get("schema_version")
        .and_then(|value| value.as_u64())
        != Some(1)
        || report.get("status").and_then(|value| value.as_str()) != Some("ready")
        || !origin.is_some_and(|origin| origin.starts_with("http://127.0.0.1:"))
    {
        return Err(ReleaseToolError::InvalidPackage(
            "demo emitted an invalid loopback readiness report".to_owned(),
        ));
    }
    Ok(())
}

fn read_line_bounded(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    for _ in 0..4096 {
        let mut byte = [0_u8; 1];
        reader.read_exact(&mut byte)?;
        result.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(result);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "readiness line exceeded 4096 bytes",
    ))
}

fn verify_fragment_files(
    fragment_path: &Path,
    version: &Version,
    bundle: &ReleaseBundleV1,
) -> Result<(), ReleaseToolError> {
    let parent = fragment_path.parent().ok_or_else(|| {
        ReleaseToolError::InvalidPackage(format!(
            "{} has no parent directory",
            fragment_path.display()
        ))
    })?;
    let bundle_path = parent.join(&bundle.name);
    let update_path = parent.join(&bundle.update_name);
    let bundle_bytes = read_regular(&bundle_path, MAX_RELEASE_FILE_BYTES)?;
    let update_bytes = read_regular(&update_path, MAX_RELEASE_FILE_BYTES)?;
    require_size_digest(&bundle_path, &bundle_bytes, bundle.size, &bundle.sha256)?;
    require_size_digest(
        &update_path,
        &update_bytes,
        bundle.update_size,
        &bundle.update_sha256,
    )?;
    let archived = read_and_validate_archive(&bundle_path, version, bundle)?;
    let suffix = executable_suffix(&bundle.target)?;
    let crawlson = archived
        .get(&format!("bin/crawlson{suffix}"))
        .ok_or_else(|| ReleaseToolError::InvalidPackage("bundle has no crawlson".to_owned()))?;
    if crawlson.as_slice() != update_bytes.as_slice() {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "{} is not byte-identical to the bundled crawlson",
            update_path.display()
        )));
    }
    Ok(())
}

fn read_and_validate_archive(
    archive_path: &Path,
    version: &Version,
    bundle: &ReleaseBundleV1,
) -> Result<BTreeMap<String, Vec<u8>>, ReleaseToolError> {
    let members = match bundle.format {
        BundleFormat::TarGz => read_tar_gz(archive_path)?,
        BundleFormat::Zip => read_zip(archive_path)?,
    };
    let root = bundle_root_name(version, &bundle.target);
    let prefix = format!("{root}/");
    let mut relative = BTreeMap::new();
    for (name, bytes) in members {
        let path = name.strip_prefix(&prefix).ok_or_else(|| {
            ReleaseToolError::InvalidPackage(format!(
                "archive member {name} is outside expected root {root}"
            ))
        })?;
        validate_relative(path)?;
        if relative.insert(path.to_owned(), bytes).is_some() {
            return Err(ReleaseContractError::Duplicate(path.to_owned()).into());
        }
    }
    let manifest_bytes = relative.get(BUNDLE_MANIFEST_NAME).ok_or_else(|| {
        ReleaseToolError::InvalidPackage(format!("archive is missing {BUNDLE_MANIFEST_NAME}"))
    })?;
    let manifest_path = archive_path.with_file_name(BUNDLE_MANIFEST_NAME);
    let manifest: BundleManifestV1 = parse_json(&manifest_path, manifest_bytes)?;
    manifest.validate()?;
    if manifest.version != *version
        || manifest.target != bundle.target
        || manifest.files != bundle.files
    {
        return Err(ReleaseToolError::InvalidPackage(
            "bundle manifest and release fragment disagree".to_owned(),
        ));
    }
    if manifest
        .files
        .iter()
        .any(|file| file.path == BUNDLE_MANIFEST_NAME)
    {
        return Err(ReleaseToolError::InvalidPackage(
            "bundle manifest cannot contain itself".to_owned(),
        ));
    }
    if relative.len() != manifest.files.len() + 1 {
        return Err(ReleaseToolError::InvalidPackage(
            "archive contains unregistered or missing members".to_owned(),
        ));
    }
    for expected in &manifest.files {
        let bytes = relative.get(&expected.path).ok_or_else(|| {
            ReleaseToolError::InvalidPackage(format!(
                "archive is missing registered member {}",
                expected.path
            ))
        })?;
        require_size_digest(archive_path, bytes, expected.size, &expected.sha256)?;
    }
    relative.remove(BUNDLE_MANIFEST_NAME);
    Ok(relative)
}

fn build_tar_gz(
    root: &str,
    files: &BTreeMap<String, ArchiveFile>,
) -> Result<Vec<u8>, ReleaseToolError> {
    let encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), Compression::best());
    let mut archive = tar::Builder::new(encoder);
    archive.mode(tar::HeaderMode::Deterministic);
    for (path, file) in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(file.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(file.bytes.len() as u64);
        header.set_cksum();
        archive
            .append_data(&mut header, format!("{root}/{path}"), file.bytes.as_slice())
            .map_err(|source| io_error(PathBuf::from(path), source))?;
    }
    let encoder = archive
        .into_inner()
        .map_err(|source| io_error(PathBuf::from("tar archive"), source))?;
    encoder
        .finish()
        .map_err(|source| io_error(PathBuf::from("gzip archive"), source))
}

fn build_zip(
    root: &str,
    files: &BTreeMap<String, ArchiveFile>,
) -> Result<Vec<u8>, ReleaseToolError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, file) in files {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::DEFAULT)
            .unix_permissions(file.mode);
        writer
            .start_file(format!("{root}/{path}"), options)
            .map_err(|source| invalid_archive(path, source))?;
        writer
            .write_all(&file.bytes)
            .map_err(|source| io_error(PathBuf::from(path), source))?;
    }
    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|source| invalid_archive("zip archive", source))
}

fn read_tar_gz(path: &Path) -> Result<BTreeMap<String, Vec<u8>>, ReleaseToolError> {
    let file = open_regular(path, MAX_RELEASE_FILE_BYTES)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut result = BTreeMap::new();
    let mut total = 0_u64;
    let entries = archive
        .entries()
        .map_err(|source| invalid_archive(&path.display().to_string(), source))?;
    for entry in entries {
        if result.len() >= MAX_ARCHIVE_ENTRIES {
            return Err(ReleaseToolError::InvalidPackage(
                "archive has too many members".to_owned(),
            ));
        }
        let mut entry = entry.map_err(|source| invalid_archive("tar member", source))?;
        if !entry.header().entry_type().is_file() {
            return Err(ReleaseToolError::InvalidPackage(
                "tar archive contains a nonregular member".to_owned(),
            ));
        }
        if entry.size() == 0 || entry.size() > MAX_RELEASE_FILE_BYTES {
            return Err(ReleaseToolError::InvalidPackage(
                "tar member size is invalid".to_owned(),
            ));
        }
        total = total.checked_add(entry.size()).ok_or_else(|| {
            ReleaseToolError::InvalidPackage("tar member sizes overflowed".to_owned())
        })?;
        if total > MAX_ARCHIVE_UNPACKED_BYTES {
            return Err(ReleaseToolError::InvalidPackage(
                "tar archive exceeds the total unpacked size limit".to_owned(),
            ));
        }
        let name = entry
            .path()
            .map_err(|source| invalid_archive("tar member path", source))?
            .to_str()
            .ok_or_else(|| {
                ReleaseToolError::InvalidPackage("tar member path is not UTF-8".to_owned())
            })?
            .to_owned();
        validate_archive_name(&name)?;
        let size = entry.size();
        let bytes = read_bounded(&mut entry, size, Path::new(&name))?;
        if result.insert(name.clone(), bytes).is_some() {
            return Err(ReleaseContractError::Duplicate(name).into());
        }
    }
    Ok(result)
}

fn read_zip(path: &Path) -> Result<BTreeMap<String, Vec<u8>>, ReleaseToolError> {
    let file = open_regular(path, MAX_RELEASE_FILE_BYTES)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|source| invalid_archive(&path.display().to_string(), source))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(ReleaseToolError::InvalidPackage(
            "archive has too many members".to_owned(),
        ));
    }
    let mut result = BTreeMap::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| invalid_archive("zip member", source))?;
        let nonregular_mode = entry.unix_mode().is_some_and(|mode| {
            let kind = mode & 0o170000;
            kind != 0 && kind != 0o100000
        });
        if !entry.is_file()
            || nonregular_mode
            || entry.size() == 0
            || entry.size() > MAX_RELEASE_FILE_BYTES
        {
            return Err(ReleaseToolError::InvalidPackage(
                "zip archive contains an invalid or nonregular member".to_owned(),
            ));
        }
        total = total.checked_add(entry.size()).ok_or_else(|| {
            ReleaseToolError::InvalidPackage("zip member sizes overflowed".to_owned())
        })?;
        if total > MAX_ARCHIVE_UNPACKED_BYTES {
            return Err(ReleaseToolError::InvalidPackage(
                "zip archive exceeds the total unpacked size limit".to_owned(),
            ));
        }
        let name = entry.name().to_owned();
        validate_archive_name(&name)?;
        let size = entry.size();
        let bytes = read_bounded(&mut entry, size, Path::new(&name))?;
        if result.insert(name.clone(), bytes).is_some() {
            return Err(ReleaseContractError::Duplicate(name).into());
        }
    }
    Ok(result)
}

fn validate_signable(path: &Path, bytes: &[u8]) -> Result<(), ReleaseToolError> {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(UPDATE_MANIFEST_NAME) => {
            let value: UpdateManifestV1 = parse_json(path, bytes)?;
            value.validate()?;
            require_complete_matrix(value.artifacts.iter().map(|item| item.target.as_str()))?;
            require_canonical(path, bytes, &value)
        }
        Some(RELEASE_INVENTORY_NAME) => {
            let value: ReleaseInventoryV1 = parse_json(path, bytes)?;
            value.validate()?;
            require_complete_matrix(value.bundles.iter().map(|item| item.target.as_str()))?;
            require_canonical(path, bytes, &value)
        }
        _ => Err(ReleaseToolError::Signing(format!(
            "{} is not a supported release manifest",
            path.display()
        ))),
    }
}

fn require_canonical<T: serde::Serialize>(
    path: &Path,
    bytes: &[u8],
    value: &T,
) -> Result<(), ReleaseToolError> {
    if canonical_json(value)? == bytes {
        Ok(())
    } else {
        Err(ReleaseToolError::Signing(format!(
            "{} is not canonical compact JSON with one trailing LF",
            path.display()
        )))
    }
}

fn verify_signature_bytes(
    public_text: &str,
    input: &[u8],
    signature_bytes: &[u8],
) -> Result<(), ReleaseToolError> {
    let signature_text = std::str::from_utf8(signature_bytes)
        .map_err(|error| ReleaseToolError::Verification(error.to_string()))?;
    let public = VerifyPublicKey::decode(public_text)
        .map_err(|error| ReleaseToolError::Verification(error.to_string()))?;
    let signature = VerifySignature::decode(signature_text)
        .map_err(|error| ReleaseToolError::Verification(error.to_string()))?;
    public
        .verify(input, &signature, false)
        .map_err(|error| ReleaseToolError::Verification(error.to_string()))
}

fn require_complete_matrix<'a>(
    targets: impl Iterator<Item = &'a str>,
) -> Result<(), ReleaseToolError> {
    let actual = targets.collect::<BTreeSet<_>>();
    let expected = SUPPORTED_RELEASE_TARGETS
        .into_iter()
        .collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(ReleaseToolError::IncompleteMatrix {
            expected: expected.into_iter().collect::<Vec<_>>().join(","),
            actual: if actual.is_empty() {
                "none".to_owned()
            } else {
                actual.into_iter().collect::<Vec<_>>().join(",")
            },
        })
    }
}

fn insert_source(
    files: &mut BTreeMap<String, ArchiveFile>,
    archive_path: String,
    source: &Path,
    mode: u32,
) -> Result<(), ReleaseToolError> {
    validate_relative(&archive_path)?;
    let bytes = read_regular(source, MAX_RELEASE_FILE_BYTES)?;
    files.insert(archive_path, ArchiveFile { bytes, mode });
    Ok(())
}

fn generated_readme(version: &Version, target: &str) -> Vec<u8> {
    format!(
        "# Crawlson {version}\n\nTarget: `{target}`\n\nAuthenticate this archive against the signed Crawlson release inventory before installation. A bundle manifest validates extracted contents but does not authenticate the archive.\n\nRun `bin/crawlson version`, `bin/clson version`, or `bin/crawlson-demo --help` from this extracted directory.\n"
    )
    .into_bytes()
}

fn current_version() -> Result<Version, ReleaseToolError> {
    let version = Version::parse(crawlson::VERSION).map_err(|error| {
        ReleaseToolError::InvalidPackage(format!("package version is invalid: {error}"))
    })?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(ReleaseContractError::UnstableVersion.into());
    }
    Ok(version)
}

fn canonical_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, ReleaseToolError> {
    let mut bytes = serde_json::to_vec(value).map_err(|source| ReleaseToolError::Json {
        path: PathBuf::from("generated JSON"),
        source,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn parse_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    bytes: &[u8],
) -> Result<T, ReleaseToolError> {
    serde_json::from_slice(bytes).map_err(|source| ReleaseToolError::Json {
        path: path.to_owned(),
        source,
    })
}

fn prepare_output_directory(path: &Path) -> Result<(), ReleaseToolError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ReleaseToolError::NotRegular(path.to_owned()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| io_error(path.to_owned(), source))
        }
        Err(source) => Err(io_error(path.to_owned(), source)),
    }
}

fn validate_directory(path: &Path) -> Result<(), ReleaseToolError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error(path.to_owned(), source))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ReleaseToolError::NotRegular(path.to_owned()));
    }
    Ok(())
}

fn open_regular(path: &Path, max: u64) -> Result<File, ReleaseToolError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error(path.to_owned(), source))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ReleaseToolError::NotRegular(path.to_owned()));
    }
    if metadata.len() == 0 {
        return Err(ReleaseToolError::Empty(path.to_owned()));
    }
    if metadata.len() > max {
        return Err(ReleaseToolError::TooLarge(path.to_owned()));
    }
    File::open(path).map_err(|source| io_error(path.to_owned(), source))
}

fn read_regular(path: &Path, max: u64) -> Result<Vec<u8>, ReleaseToolError> {
    let mut file = open_regular(path, max)?;
    let size = file
        .metadata()
        .map_err(|source| io_error(path.to_owned(), source))?
        .len();
    read_bounded(&mut file, size, path)
}

fn read_utf8_regular(path: &Path, max: u64) -> Result<String, ReleaseToolError> {
    String::from_utf8(read_regular(path, max)?)
        .map_err(|error| ReleaseToolError::Signing(error.to_string()))
}

fn read_bounded(
    reader: &mut impl Read,
    expected: u64,
    path: &Path,
) -> Result<Vec<u8>, ReleaseToolError> {
    let mut bytes = Vec::with_capacity(usize::try_from(expected).unwrap_or(0));
    reader
        .take(expected.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path.to_owned(), source))?;
    if bytes.len() as u64 != expected {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "{} changed size while being read",
            path.display()
        )));
    }
    Ok(bytes)
}

fn require_size_digest(
    path: &Path,
    bytes: &[u8],
    size: u64,
    sha256: &str,
) -> Result<(), ReleaseToolError> {
    if bytes.len() as u64 != size || digest(bytes) != sha256 {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "{} does not match its declared size and SHA-256",
            path.display()
        )));
    }
    Ok(())
}

fn enforce_generated_size(name: &str, bytes: &[u8]) -> Result<(), ReleaseToolError> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_RELEASE_FILE_BYTES {
        Err(ReleaseToolError::InvalidPackage(format!(
            "generated {name} is outside the supported size range"
        )))
    } else {
        Ok(())
    }
}

fn publish_files(files: &[(PathBuf, Vec<u8>)]) -> Result<(), ReleaseToolError> {
    ensure_outputs_absent(files.iter().map(|(path, _)| path.as_path()))?;
    let mut created = Vec::new();
    for (path, bytes) in files {
        let result = write_new(path, bytes, false);
        match result {
            Ok(()) => created.push(path.clone()),
            Err(error) => {
                for created_path in created {
                    let _ = fs::remove_file(created_path);
                }
                return Err(error);
            }
        }
    }
    Ok(())
}

fn publish_test_key_pair(
    public_path: &Path,
    public_bytes: &[u8],
    secret_path: &Path,
    secret_bytes: &[u8],
) -> Result<(), ReleaseToolError> {
    ensure_outputs_absent([public_path, secret_path])?;
    write_new(secret_path, secret_bytes, true)?;
    if let Err(error) = write_new(public_path, public_bytes, false) {
        let _ = fs::remove_file(secret_path);
        return Err(error);
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8], secret: bool) -> Result<(), ReleaseToolError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if secret {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = secret;
    let mut file = options.open(path).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            ReleaseToolError::OutputExists(path.to_owned())
        } else {
            io_error(path.to_owned(), source)
        }
    })?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| io_error(path.to_owned(), source));
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn ensure_outputs_absent<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), ReleaseToolError> {
    for path in paths {
        match fs::symlink_metadata(path) {
            Ok(_) => return Err(ReleaseToolError::OutputExists(path.to_owned())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(path.to_owned(), source)),
        }
    }
    Ok(())
}

fn validate_archive_name(path: &str) -> Result<(), ReleaseToolError> {
    validate_relative(path)?;
    if !path.contains('/') {
        return Err(ReleaseToolError::InvalidPackage(format!(
            "archive member has no versioned root: {path}"
        )));
    }
    Ok(())
}

fn validate_relative(path: &str) -> Result<(), ReleaseToolError> {
    let parsed = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        Err(ReleaseContractError::UnsafePath(path.to_owned()).into())
    } else {
        Ok(())
    }
}

fn signature_path(input: &Path) -> Result<PathBuf, ReleaseToolError> {
    let name = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ReleaseToolError::Signing(format!("{} has no UTF-8 file name", input.display()))
        })?;
    let signature_name = match name {
        UPDATE_MANIFEST_NAME => UPDATE_SIGNATURE_NAME,
        RELEASE_INVENTORY_NAME => RELEASE_SIGNATURE_NAME,
        _ => return Err(ReleaseToolError::Signing(format!("{name} is not signable"))),
    };
    Ok(input.with_file_name(signature_name))
}

fn sibling_public_key(secret: &Path) -> PathBuf {
    if secret.file_name().and_then(|name| name.to_str()) == Some(TEST_SECRET_KEY_NAME) {
        secret.with_file_name(TEST_PUBLIC_KEY_NAME)
    } else {
        secret.with_extension("pub")
    }
}

fn expected_targets() -> String {
    SUPPORTED_RELEASE_TARGETS.join(",")
}

fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut result = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}

fn with_one_lf(mut bytes: Vec<u8>) -> Vec<u8> {
    while bytes.last() == Some(&b'\n') || bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    bytes.push(b'\n');
    bytes
}

fn io_error(path: PathBuf, source: io::Error) -> ReleaseToolError {
    ReleaseToolError::Io { path, source }
}

fn invalid_archive(name: &str, error: impl std::fmt::Display) -> ReleaseToolError {
    ReleaseToolError::InvalidPackage(format!("{name}: {error}"))
}

fn signing_error(error: impl std::fmt::Display) -> ReleaseToolError {
    ReleaseToolError::Signing(error.to_string())
}

pub fn main_entry() -> ExitCode {
    match crate::cli::run() {
        Ok(paths) => {
            for path in paths {
                println!("{}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("crawlson-release: {error}");
            ExitCode::FAILURE
        }
    }
}

mod cli {
    use clap::{Parser, Subcommand};

    use super::*;

    #[derive(Debug, Parser)]
    #[command(
        name = "crawlson-release",
        version,
        about = "Build and verify private Crawlson release dry-run artifacts"
    )]
    struct Arguments {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand)]
    enum Command {
        /// Build one deterministic target bundle, raw update payload, and fragment.
        Package {
            #[arg(long)]
            target: String,
            #[arg(long)]
            bin_dir: PathBuf,
            #[arg(long, default_value = ".")]
            source_dir: PathBuf,
            #[arg(long)]
            out_dir: PathBuf,
        },
        /// Validate and assemble exactly one fragment for every supported target.
        Assemble {
            #[arg(long, required = true)]
            input: Vec<PathBuf>,
            #[arg(long)]
            out_dir: PathBuf,
        },
        /// Generate a disposable encrypted Minisign key pair for a private dry run.
        GenerateTestKey {
            #[arg(long)]
            out_dir: PathBuf,
        },
        /// Sign and immediately verify canonical complete release manifests.
        Sign {
            #[arg(long)]
            secret_key: PathBuf,
            #[arg(long)]
            public_key: Option<PathBuf>,
            #[arg(long, required = true)]
            input: Vec<PathBuf>,
        },
    }

    pub fn run() -> Result<Vec<PathBuf>, ReleaseToolError> {
        match Arguments::parse().command {
            Command::Package {
                target,
                bin_dir,
                source_dir,
                out_dir,
            } => {
                let output = package(&PackageOptions {
                    target,
                    bin_dir,
                    source_dir,
                    out_dir,
                })?;
                Ok(vec![output.bundle, output.update, output.fragment])
            }
            Command::Assemble { input, out_dir } => {
                let output = assemble(&AssembleOptions {
                    inputs: input,
                    out_dir,
                })?;
                Ok(vec![output.update_manifest, output.release_inventory])
            }
            Command::GenerateTestKey { out_dir } => {
                let output = generate_test_key(&out_dir)?;
                Ok(vec![output.public_key, output.secret_key])
            }
            Command::Sign {
                secret_key,
                public_key,
                input,
            } => sign(&SignOptions {
                secret_key,
                public_key,
                inputs: input,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        bin: PathBuf,
        source: PathBuf,
    }

    #[test]
    fn unix_and_windows_packages_are_byte_reproducible() {
        for target in ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"] {
            let fixture = fixture(target);
            let first = fixture._root.path().join("first");
            let second = fixture._root.path().join("second");
            let first_outputs = package_fixture(&PackageOptions {
                target: target.to_owned(),
                bin_dir: fixture.bin.clone(),
                source_dir: fixture.source.clone(),
                out_dir: first,
            })
            .unwrap();
            let second_outputs = package_fixture(&PackageOptions {
                target: target.to_owned(),
                bin_dir: fixture.bin.clone(),
                source_dir: fixture.source.clone(),
                out_dir: second,
            })
            .unwrap();

            assert_eq!(
                fs::read(first_outputs.bundle).unwrap(),
                fs::read(second_outputs.bundle).unwrap(),
                "{target} bundle"
            );
            assert_eq!(
                fs::read(first_outputs.update).unwrap(),
                fs::read(second_outputs.update).unwrap(),
                "{target} updater"
            );
            assert_eq!(
                fs::read(first_outputs.fragment).unwrap(),
                fs::read(second_outputs.fragment).unwrap(),
                "{target} fragment"
            );
        }
    }

    #[test]
    fn package_contains_only_registered_normalized_regular_files() {
        for target in ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"] {
            let fixture = fixture(target);
            let outputs = package_fixture(&PackageOptions {
                target: target.to_owned(),
                bin_dir: fixture.bin.clone(),
                source_dir: fixture.source.clone(),
                out_dir: fixture._root.path().join("out"),
            })
            .unwrap();
            let fragment: ReleaseInventoryV1 =
                serde_json::from_slice(&fs::read(&outputs.fragment).unwrap()).unwrap();
            let bundle = &fragment.bundles[0];
            let members = read_and_validate_archive(&outputs.bundle, &fragment.version, bundle)
                .expect("the generated archive must round trip");
            assert_eq!(
                members
                    .get(&format!(
                        "bin/crawlson{}",
                        executable_suffix(target).unwrap()
                    ))
                    .unwrap(),
                &fs::read(outputs.update).unwrap()
            );
            assert!(members.contains_key("README.md"));
            assert!(members.contains_key("examples/demo-pass.toml"));
            assert!(members.contains_key("examples/demo-fail.toml"));
            assert!(members.contains_key("scripts/demo.sh"));
        }
    }

    #[test]
    fn assemble_requires_and_canonically_orders_the_exact_matrix() {
        let root = tempfile::tempdir().unwrap();
        let mut fragments = Vec::new();
        for target in SUPPORTED_RELEASE_TARGETS.into_iter().rev() {
            let fixture = fixture(target);
            let output = package_fixture(&PackageOptions {
                target: target.to_owned(),
                bin_dir: fixture.bin,
                source_dir: fixture.source,
                out_dir: root.path().join(target),
            })
            .unwrap();
            fragments.push(output.fragment);
        }
        let outputs = assemble(&AssembleOptions {
            inputs: fragments,
            out_dir: root.path().join("assembled"),
        })
        .unwrap();
        let update_bytes = fs::read(&outputs.update_manifest).unwrap();
        let inventory_bytes = fs::read(&outputs.release_inventory).unwrap();
        assert_eq!(update_bytes.last(), Some(&b'\n'));
        assert_eq!(inventory_bytes.last(), Some(&b'\n'));
        assert!(!update_bytes[..update_bytes.len() - 1].contains(&b'\n'));
        assert!(!inventory_bytes[..inventory_bytes.len() - 1].contains(&b'\n'));

        let update: UpdateManifestV1 = serde_json::from_slice(&update_bytes).unwrap();
        let inventory: ReleaseInventoryV1 = serde_json::from_slice(&inventory_bytes).unwrap();
        assert_eq!(update.schema_version, 1);
        assert_eq!(inventory.schema_version, 1);
        assert_eq!(update.artifacts.len(), 4);
        assert_eq!(inventory.bundles.len(), 4);
        let targets = update
            .artifacts
            .iter()
            .map(|artifact| artifact.target.as_str())
            .collect::<Vec<_>>();
        assert_eq!(targets, SUPPORTED_RELEASE_TARGETS);
        assert!(update.artifacts.iter().all(
            |artifact| !artifact.name.ends_with(".zip") && !artifact.name.ends_with(".tar.gz")
        ));
        validate_schema("update-manifest-v1.schema.json", &update_bytes);
        validate_schema("release-inventory-v1.schema.json", &inventory_bytes);
        let mut invalid_update: serde_json::Value = serde_json::from_slice(&update_bytes).unwrap();
        let windows = invalid_update["artifacts"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|artifact| artifact["target"] == "x86_64-pc-windows-msvc")
            .unwrap();
        windows["name"] = serde_json::Value::String(
            windows["name"]
                .as_str()
                .unwrap()
                .trim_end_matches(".exe")
                .to_owned(),
        );
        assert_schema_rejects("update-manifest-v1.schema.json", &invalid_update);
        let mut invalid_inventory: serde_json::Value =
            serde_json::from_slice(&inventory_bytes).unwrap();
        let windows = invalid_inventory["bundles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|bundle| bundle["target"] == "x86_64-pc-windows-msvc")
            .unwrap();
        windows["format"] = serde_json::Value::String("tar_gz".to_owned());
        assert_schema_rejects("release-inventory-v1.schema.json", &invalid_inventory);

        let bundle_manifest = BundleManifestV1 {
            schema_version: 1,
            version: inventory.version.clone(),
            target: inventory.bundles[0].target.clone(),
            files: inventory.bundles[0].files.clone(),
        };
        validate_schema(
            "crawlson-bundle-v1.schema.json",
            &canonical_json(&bundle_manifest).unwrap(),
        );
    }

    #[test]
    fn assembly_rejects_missing_duplicate_and_tampered_inputs_without_outputs() {
        let root = tempfile::tempdir().unwrap();
        let fixture = fixture(SUPPORTED_RELEASE_TARGETS[0]);
        let package = package_fixture(&PackageOptions {
            target: SUPPORTED_RELEASE_TARGETS[0].to_owned(),
            bin_dir: fixture.bin,
            source_dir: fixture.source,
            out_dir: root.path().join("one"),
        })
        .unwrap();
        let incomplete_out = root.path().join("incomplete");
        assert!(
            assemble(&AssembleOptions {
                inputs: vec![package.fragment.clone()],
                out_dir: incomplete_out.clone(),
            })
            .is_err()
        );
        assert!(!incomplete_out.join(UPDATE_MANIFEST_NAME).exists());
        assert!(
            assemble(&AssembleOptions {
                inputs: vec![package.fragment.clone(), package.fragment.clone()],
                out_dir: root.path().join("duplicate"),
            })
            .is_err()
        );
        let mut update = OpenOptions::new()
            .append(true)
            .open(&package.update)
            .unwrap();
        update.write_all(b"tampered").unwrap();
        assert!(
            assemble(&AssembleOptions {
                inputs: vec![package.fragment],
                out_dir: root.path().join("tampered"),
            })
            .is_err()
        );
    }

    #[test]
    fn package_never_overwrites_and_rejects_nonregular_inputs() {
        let target = "x86_64-unknown-linux-gnu";
        let fixture = fixture(target);
        let options = PackageOptions {
            target: target.to_owned(),
            bin_dir: fixture.bin.clone(),
            source_dir: fixture.source.clone(),
            out_dir: fixture._root.path().join("out"),
        };
        package_fixture(&options).unwrap();
        let before = directory_snapshot(&options.out_dir);
        assert!(matches!(
            package_fixture(&options),
            Err(ReleaseToolError::OutputExists(_))
        ));
        assert_eq!(before, directory_snapshot(&options.out_dir));

        fs::remove_file(fixture.bin.join("clson")).unwrap();
        fs::create_dir(fixture.bin.join("clson")).unwrap();
        let bad = PackageOptions {
            out_dir: fixture._root.path().join("bad"),
            ..options
        };
        assert!(matches!(
            package_fixture(&bad),
            Err(ReleaseToolError::NotRegular(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn package_rejects_symlinked_source_files() {
        use std::os::unix::fs::symlink;

        let target = "x86_64-unknown-linux-gnu";
        let fixture = fixture(target);
        let real = fixture.bin.join("real-crawlson");
        fs::rename(fixture.bin.join("crawlson"), &real).unwrap();
        symlink(&real, fixture.bin.join("crawlson")).unwrap();
        assert!(matches!(
            package_fixture(&PackageOptions {
                target: target.to_owned(),
                bin_dir: fixture.bin,
                source_dir: fixture.source,
                out_dir: fixture._root.path().join("out"),
            }),
            Err(ReleaseToolError::NotRegular(_))
        ));
    }

    #[test]
    fn test_only_key_signs_both_manifests_and_updater_verifier_rejects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let assembled = complete_assembly(root.path());
        let keys = generate_test_key(&root.path().join("keys")).unwrap();
        let signatures = sign(&SignOptions {
            secret_key: keys.secret_key.clone(),
            public_key: None,
            inputs: vec![
                assembled.update_manifest.clone(),
                assembled.release_inventory.clone(),
            ],
        })
        .unwrap();
        assert_eq!(signatures.len(), 2);
        for (input, signature) in [
            (&assembled.update_manifest, &signatures[0]),
            (&assembled.release_inventory, &signatures[1]),
        ] {
            verify_signature(&keys.public_key, input, signature).unwrap();
        }

        let mut bytes = fs::read(&assembled.update_manifest).unwrap();
        bytes[0] ^= 1;
        fs::write(&assembled.update_manifest, bytes).unwrap();
        assert!(
            verify_signature(&keys.public_key, &assembled.update_manifest, &signatures[0]).is_err()
        );
        assert!(matches!(
            sign(&SignOptions {
                secret_key: keys.secret_key,
                public_key: Some(keys.public_key),
                inputs: vec![assembled.release_inventory],
            }),
            Err(ReleaseToolError::OutputExists(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_secret_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let keys = generate_test_key(root.path()).unwrap();
        assert_eq!(
            fs::metadata(keys.secret_key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    fn complete_assembly(root: &Path) -> AssembleOutputs {
        let mut inputs = Vec::new();
        for target in SUPPORTED_RELEASE_TARGETS {
            let fixture = fixture(target);
            let outputs = package_fixture(&PackageOptions {
                target: target.to_owned(),
                bin_dir: fixture.bin,
                source_dir: fixture.source,
                out_dir: root.join(target),
            })
            .unwrap();
            inputs.push(outputs.fragment);
        }
        assemble(&AssembleOptions {
            inputs,
            out_dir: root.join("assembled"),
        })
        .unwrap()
    }

    fn package_fixture(options: &PackageOptions) -> Result<PackageOutputs, ReleaseToolError> {
        package_impl(options, false)
    }

    fn fixture(target: &str) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let source = root.path().join("source");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(source.join("examples")).unwrap();
        fs::create_dir_all(source.join("scripts")).unwrap();
        let suffix = executable_suffix(target).unwrap();
        for name in ["crawlson", "clson", "crawlson-demo"] {
            fs::write(
                bin.join(format!("{name}{suffix}")),
                format!("fixture:{target}:{name}\n"),
            )
            .unwrap();
        }
        fs::write(source.join("examples/demo-pass.toml"), b"pass = true\n").unwrap();
        fs::write(source.join("examples/demo-fail.toml"), b"pass = false\n").unwrap();
        fs::write(
            source.join("scripts/demo.sh"),
            b"#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();
        Fixture {
            _root: root,
            bin,
            source,
        }
    }

    fn validate_schema(name: &str, instance: &[u8]) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let schema: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("schemas").join(name)).unwrap()).unwrap();
        jsonschema::meta::validate(&schema).unwrap();
        let value: serde_json::Value = serde_json::from_slice(instance).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap();
    }

    fn assert_schema_rejects(name: &str, instance: &serde_json::Value) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let schema: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("schemas").join(name)).unwrap()).unwrap();
        jsonschema::meta::validate(&schema).unwrap();
        assert!(
            !jsonschema::validator_for(&schema)
                .unwrap()
                .is_valid(instance)
        );
    }

    fn directory_snapshot(path: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut result = BTreeMap::new();
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let mut bytes = Vec::new();
            File::open(entry.path())
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            result.insert(entry.file_name().to_string_lossy().into_owned(), bytes);
        }
        result
    }
}
