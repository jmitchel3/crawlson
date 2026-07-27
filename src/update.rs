use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{BUILD_TARGET, CommandResult, VERSION};

const RELEASE_API: &str = "https://api.github.com/repos/jmitchel3/crawlson/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/jmitchel3/crawlson/releases/download/";
const MANIFEST_NAME: &str = "crawlson-update.json";
const SIGNATURE_NAME: &str = "crawlson-update.json.minisig";
const UPDATE_PUBLIC_KEY: Option<&str> = option_env!("CRAWLSON_UPDATE_PUBLIC_KEY");
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const CHECK_SUCCESS_INTERVAL: u64 = 7 * 24 * 60 * 60;
const CHECK_FAILURE_INTERVAL: u64 = 24 * 60 * 60;
const SUCCESS_JITTER_MAX: u64 = 48 * 60 * 60;
const FAILURE_JITTER_MAX: u64 = 6 * 60 * 60;

#[derive(Debug, Clone, Copy)]
pub struct ManualUpgradeOptions {
    pub check_only: bool,
    pub offline: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerifiedCandidate {
    pub version: Version,
    pub target: String,
    pub asset_name: String,
    pub size: u64,
    pub sha256: String,
    pub release_url: String,
    #[serde(skip)]
    download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedInstall {
    pub binary: PathBuf,
    pub install_id: String,
}

pub trait UpdateBackend {
    fn check(&self, target: &str) -> Result<Option<VerifiedCandidate>, UpdateError>;
    fn install(
        &self,
        candidate: &VerifiedCandidate,
        install: &ManagedInstall,
    ) -> Result<(), UpdateError>;
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("signed Crawlson releases are not configured in this build")]
    SigningNotConfigured,
    #[error("release request failed: {0}")]
    Request(String),
    #[error("release metadata was invalid: {0}")]
    InvalidMetadata(String),
    #[error("release signature verification failed: {0}")]
    InvalidSignature(String),
    #[error("release does not contain an update for target {0}")]
    UnsupportedTarget(String),
    #[error("update download failed: {0}")]
    Download(String),
    #[error("downloaded artifact failed verification: {0}")]
    Verification(String),
    #[error("could not replace the Crawlson executable: {0}")]
    Replacement(String),
    #[error("update state failed: {0}")]
    State(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    Auto,
    Notify,
    Off,
}

#[derive(Debug, Clone, Serialize)]
struct UpgradeReport {
    schema_version: u8,
    status: UpgradeStatus,
    current_version: Version,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<Version>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_url: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum UpgradeStatus {
    UpToDate,
    UpdateAvailable,
    Upgraded,
    Blocked,
    Error,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    immutable: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    state: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    schema_version: u8,
    version: Version,
    artifacts: Vec<ManifestArtifact>,
}

#[derive(Debug, Deserialize)]
struct ManifestArtifact {
    target: String,
    name: String,
    size: u64,
    sha256: String,
}

#[derive(Debug)]
struct GithubSignedBackend {
    public_key: Option<&'static str>,
    metadata_agent: ureq::Agent,
    artifact_agent: ureq::Agent,
}

impl GithubSignedBackend {
    fn new() -> Self {
        let metadata_config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .timeout_connect(Some(Duration::from_secs(5)))
            .user_agent(format!("crawlson/{VERSION}"))
            .build();
        let artifact_config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5 * 60)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .user_agent(format!("crawlson/{VERSION}"))
            .build();
        Self {
            public_key: UPDATE_PUBLIC_KEY,
            metadata_agent: metadata_config.into(),
            artifact_agent: artifact_config.into(),
        }
    }

    fn get_release(&self) -> Result<Option<GithubRelease>, UpdateError> {
        let response = match self
            .metadata_agent
            .get(RELEASE_API)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10")
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(404)) => return Ok(None),
            Err(error) => return Err(UpdateError::Request(error.to_string())),
        };
        response
            .into_body()
            .with_config()
            .limit(MAX_METADATA_BYTES)
            .read_json()
            .map(Some)
            .map_err(|error| UpdateError::InvalidMetadata(error.to_string()))
    }

    fn download_metadata(&self, asset: &GithubAsset) -> Result<Vec<u8>, UpdateError> {
        validate_download_url(&asset.browser_download_url)?;
        if asset.size == 0 || asset.size > MAX_METADATA_BYTES {
            return Err(UpdateError::InvalidMetadata(format!(
                "{} has invalid size {}",
                asset.name, asset.size
            )));
        }
        let response = self
            .metadata_agent
            .get(&asset.browser_download_url)
            .call()
            .map_err(|error| UpdateError::Request(error.to_string()))?;
        let bytes = response
            .into_body()
            .with_config()
            .limit(MAX_METADATA_BYTES)
            .read_to_vec()
            .map_err(|error| UpdateError::Request(error.to_string()))?;
        if u64::try_from(bytes.len()).ok() != Some(asset.size) {
            return Err(UpdateError::InvalidMetadata(format!(
                "{} size did not match release metadata",
                asset.name
            )));
        }
        verify_bytes_digest(&bytes, asset.digest.as_deref(), &asset.name)?;
        Ok(bytes)
    }
}

impl UpdateBackend for GithubSignedBackend {
    fn check(&self, target: &str) -> Result<Option<VerifiedCandidate>, UpdateError> {
        let public_key_text = self.public_key.ok_or(UpdateError::SigningNotConfigured)?;
        let release = match self.get_release()? {
            Some(release) => release,
            None => return Ok(None),
        };
        if release.draft || release.prerelease || !release.immutable {
            return Err(UpdateError::InvalidMetadata(
                "the latest release is not a stable immutable release".to_owned(),
            ));
        }

        let manifest_asset = find_uploaded_asset(&release.assets, MANIFEST_NAME)?;
        let signature_asset = find_uploaded_asset(&release.assets, SIGNATURE_NAME)?;
        let manifest_bytes = self.download_metadata(manifest_asset)?;
        let signature_bytes = self.download_metadata(signature_asset)?;
        verify_manifest_signature(public_key_text, &manifest_bytes, &signature_bytes)?;

        let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| UpdateError::InvalidMetadata(error.to_string()))?;
        verified_candidate_from_manifest(&release, &manifest, target).map(Some)
    }

    fn install(
        &self,
        candidate: &VerifiedCandidate,
        install: &ManagedInstall,
    ) -> Result<(), UpdateError> {
        let current = env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|error| UpdateError::Replacement(error.to_string()))?;
        if current != install.binary {
            return Err(UpdateError::Replacement(
                "managed-install receipt does not match the running crawlson binary".to_owned(),
            ));
        }
        let parent = current.parent().ok_or_else(|| {
            UpdateError::Replacement("the current executable has no parent directory".to_owned())
        })?;
        let mut temporary = NamedTempFile::new_in(parent)
            .map_err(|error| UpdateError::Download(error.to_string()))?;
        let response = self
            .artifact_agent
            .get(&candidate.download_url)
            .call()
            .map_err(|error| UpdateError::Download(error.to_string()))?;
        let mut reader = response.into_body().into_reader();
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| UpdateError::Download(error.to_string()))?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > candidate.size || total > MAX_BINARY_BYTES {
                return Err(UpdateError::Verification(
                    "download exceeded the signed size".to_owned(),
                ));
            }
            hasher.update(&buffer[..read]);
            temporary
                .write_all(&buffer[..read])
                .map_err(|error| UpdateError::Download(error.to_string()))?;
        }
        if total != candidate.size {
            return Err(UpdateError::Verification(
                "download size did not match the signed manifest".to_owned(),
            ));
        }
        let actual_digest = hex_digest(hasher.finalize());
        if actual_digest != candidate.sha256 {
            return Err(UpdateError::Verification(
                "download digest did not match the signed manifest".to_owned(),
            ));
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| UpdateError::Download(error.to_string()))?;
        make_executable(temporary.path())?;
        replace_executable(temporary.path())
    }
}

pub fn run_manual(options: ManualUpgradeOptions) -> CommandResult {
    let backend = GithubSignedBackend::new();
    run_manual_with_backend(options, &backend, installation_ownership())
}

fn run_manual_with_backend(
    options: ManualUpgradeOptions,
    backend: &dyn UpdateBackend,
    ownership: InstallOwnership,
) -> CommandResult {
    let current = Version::parse(VERSION).expect("Cargo package version is valid semver");
    if options.offline || env_truthy("CRAWLSON_OFFLINE") {
        return render_upgrade(
            options.json,
            1,
            UpgradeReport {
                schema_version: 1,
                status: UpgradeStatus::Blocked,
                current_version: current,
                latest_version: None,
                release_url: None,
                message: "upgrade is unavailable in offline mode".to_owned(),
            },
        );
    }

    let candidate = match backend.check(BUILD_TARGET) {
        Ok(candidate) => candidate,
        Err(error) => {
            return render_upgrade(
                options.json,
                1,
                UpgradeReport {
                    schema_version: 1,
                    status: UpgradeStatus::Error,
                    current_version: current,
                    latest_version: None,
                    release_url: None,
                    message: error.to_string(),
                },
            );
        }
    };
    let Some(candidate) = candidate else {
        return render_upgrade(
            options.json,
            0,
            UpgradeReport {
                schema_version: 1,
                status: UpgradeStatus::UpToDate,
                current_version: current,
                latest_version: None,
                release_url: None,
                message: "no stable Crawlson release is published yet".to_owned(),
            },
        );
    };

    if let Err(message) = validate_candidate_version(&current, &candidate.version) {
        let up_to_date = candidate.version == current;
        return render_upgrade(
            options.json,
            if up_to_date { 0 } else { 1 },
            UpgradeReport {
                schema_version: 1,
                status: if up_to_date {
                    UpgradeStatus::UpToDate
                } else {
                    UpgradeStatus::Blocked
                },
                current_version: current,
                latest_version: Some(candidate.version),
                release_url: Some(candidate.release_url),
                message,
            },
        );
    }

    if options.check_only {
        return render_upgrade(
            options.json,
            0,
            UpgradeReport {
                schema_version: 1,
                status: UpgradeStatus::UpdateAvailable,
                current_version: current,
                latest_version: Some(candidate.version),
                release_url: Some(candidate.release_url),
                message: "a newer signed release is available".to_owned(),
            },
        );
    }

    let install = match ownership {
        InstallOwnership::Standalone(install) => install,
        InstallOwnership::PackageManager { hint } => {
            return render_upgrade(
                options.json,
                1,
                UpgradeReport {
                    schema_version: 1,
                    status: UpgradeStatus::Blocked,
                    current_version: current,
                    latest_version: Some(candidate.version),
                    release_url: Some(candidate.release_url),
                    message: format!(
                        "this installation is package-managed; upgrade it with: {hint}"
                    ),
                },
            );
        }
        InstallOwnership::Unknown => {
            return render_upgrade(
                options.json,
                1,
                UpgradeReport {
                    schema_version: 1,
                    status: UpgradeStatus::Blocked,
                    current_version: current,
                    latest_version: Some(candidate.version),
                    release_url: Some(candidate.release_url),
                    message: "self-upgrade requires a first-party managed-install receipt"
                        .to_owned(),
                },
            );
        }
    };

    match backend.install(&candidate, &install) {
        Ok(()) => render_upgrade(
            options.json,
            0,
            UpgradeReport {
                schema_version: 1,
                status: UpgradeStatus::Upgraded,
                current_version: current,
                latest_version: Some(candidate.version.clone()),
                release_url: Some(candidate.release_url),
                message: format!("upgraded Crawlson to {}", candidate.version),
            },
        ),
        Err(error) => render_upgrade(
            options.json,
            1,
            UpgradeReport {
                schema_version: 1,
                status: UpgradeStatus::Error,
                current_version: current,
                latest_version: Some(candidate.version),
                release_url: Some(candidate.release_url),
                message: error.to_string(),
            },
        ),
    }
}

fn render_upgrade(json: bool, exit_code: u8, report: UpgradeReport) -> CommandResult {
    if json {
        let mut stdout = serde_json::to_string(&report).expect("upgrade report is serializable");
        stdout.push('\n');
        CommandResult {
            exit_code,
            stdout,
            stderr: String::new(),
        }
    } else {
        let message = format!("{}\n", report.message);
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

pub fn finish_foreground(result: &mut CommandResult, show_notice: bool) {
    if show_notice && let Some(notice) = cached_notice() {
        result.stderr.push_str(&notice);
        result.stderr.push('\n');
    }
    spawn_periodic_worker_if_due();
}

pub fn spawn_periodic_worker_if_due() {
    if !periodic_allowed(
        PeriodicContext::from_env(),
        configured_mode(installation_ownership()),
    ) || UPDATE_PUBLIC_KEY.is_none()
    {
        return;
    }
    let paths = match UpdatePaths::discover() {
        Some(paths) => paths,
        None => return,
    };
    let now = unix_now();
    let state = read_state(&paths.state).unwrap_or_else(|_| UpdateState::new(None));
    if state.next_check_at.is_some_and(|next| next > now) {
        return;
    }
    let executable = match env::current_exe() {
        Ok(executable) => executable,
        Err(_) => return,
    };
    let _ = Command::new(executable)
        .arg("__update-worker")
        .env("CRAWLSON_UPDATE_WORKER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn run_periodic_worker() -> CommandResult {
    if env::var_os("CRAWLSON_UPDATE_WORKER").is_none()
        || !periodic_allowed(
            PeriodicContext::from_env(),
            configured_mode(installation_ownership()),
        )
    {
        return CommandResult::success(String::new());
    }
    let Some(paths) = UpdatePaths::discover() else {
        return CommandResult::success(String::new());
    };
    periodic_worker_command(&paths, unix_now(), &GithubSignedBackend::new())
}

fn periodic_worker_command(
    paths: &UpdatePaths,
    now: u64,
    backend: &dyn UpdateBackend,
) -> CommandResult {
    let _ = periodic_worker(paths, now, backend);
    CommandResult::success(String::new())
}

fn periodic_worker(
    paths: &UpdatePaths,
    now: u64,
    backend: &dyn UpdateBackend,
) -> Result<(), UpdateError> {
    if let Some(parent) = paths.lock.parent() {
        fs::create_dir_all(parent).map_err(state_error)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)
        .map_err(state_error)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Ok(()),
        Err(fs::TryLockError::Error(error)) => return Err(state_error(error)),
    }

    let ownership = installation_ownership();
    let install_id = match &ownership {
        InstallOwnership::Standalone(install) => Some(install.install_id.as_str()),
        InstallOwnership::PackageManager { .. } | InstallOwnership::Unknown => None,
    };
    let mut state = read_state(&paths.state)
        .ok()
        .filter(|state| install_id.is_none_or(|expected| state.install_id == expected))
        .unwrap_or_else(|| UpdateState::new(install_id));
    if state.next_check_at.is_some_and(|next| next > now) {
        return Ok(());
    }
    state.last_attempt_at = Some(now);
    state.next_check_at = Some(now + failure_delay(&state.install_id, now));
    write_state(&paths.state, &state)?;

    let current = Version::parse(VERSION).expect("Cargo package version is valid semver");
    match backend.check(BUILD_TARGET) {
        Ok(candidate) => {
            state.failure_count = 0;
            state.last_success_at = Some(now);
            state.next_check_at = Some(now + success_delay(&state.install_id, now));
            if let Some(candidate) = candidate {
                state.latest_seen = Some(candidate.version.clone());
                let mode = configured_mode(ownership.clone());
                if mode == UpdateMode::Auto
                    && auto_compatible(&current, &candidate.version)
                    && validate_candidate_version(&current, &candidate.version).is_ok()
                    && state
                        .last_install_at
                        .is_none_or(|last| now.saturating_sub(last) >= CHECK_SUCCESS_INTERVAL)
                    && let InstallOwnership::Standalone(install) = ownership
                {
                    backend.install(&candidate, &install)?;
                    state.last_install_at = Some(now);
                    state.latest_seen = Some(candidate.version);
                }
            }
        }
        Err(error) => {
            state.failure_count = state.failure_count.saturating_add(1).min(8);
            write_state(&paths.state, &state)?;
            return Err(error);
        }
    }
    write_state(&paths.state, &state)
}

fn cached_notice() -> Option<String> {
    if !periodic_allowed(
        PeriodicContext::from_env(),
        configured_mode(installation_ownership()),
    ) {
        return None;
    }
    let paths = UpdatePaths::discover()?;
    let state = read_state(&paths.state).ok()?;
    let latest = state.latest_seen?;
    let current = Version::parse(VERSION).ok()?;
    (latest > current).then(|| format!("Crawlson {latest} is available; run 'crawlson upgrade'"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpdateState {
    schema_version: u8,
    install_id: String,
    next_check_at: Option<u64>,
    last_attempt_at: Option<u64>,
    last_success_at: Option<u64>,
    last_install_at: Option<u64>,
    latest_seen: Option<Version>,
    failure_count: u8,
}

impl UpdateState {
    fn new(managed_install_id: Option<&str>) -> Self {
        Self {
            schema_version: 1,
            install_id: managed_install_id
                .map(str::to_owned)
                .unwrap_or_else(generated_install_id),
            next_check_at: None,
            last_attempt_at: None,
            last_success_at: None,
            last_install_at: None,
            latest_seen: None,
            failure_count: 0,
        }
    }
}

fn generated_install_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let executable = env::current_exe().unwrap_or_default();
    let seed = format!("{}:{}:{nanos}", executable.display(), std::process::id());
    format!(
        "{:016x}{:016x}",
        jitter_hash(&seed, nanos as u64),
        jitter_hash(&seed, (nanos >> 64) as u64)
    )
}

#[derive(Debug)]
struct UpdatePaths {
    config: PathBuf,
    receipt: PathBuf,
    state: PathBuf,
    lock: PathBuf,
}

impl UpdatePaths {
    fn discover() -> Option<Self> {
        if let Some(root) = env::var_os("CRAWLSON_HOME") {
            let root = PathBuf::from(root);
            return Some(Self {
                config: root.join("config.toml"),
                receipt: root.join("install.json"),
                state: root.join("update-state.json"),
                lock: root.join("update.lock"),
            });
        }
        let dirs = ProjectDirs::from("org", "crawlson", "crawlson")?;
        let state_root = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
        Some(Self {
            config: dirs.config_dir().join("config.toml"),
            receipt: dirs.data_local_dir().join("install.json"),
            state: state_root.join("update-state.json"),
            lock: state_root.join("update.lock"),
        })
    }
}

#[derive(Debug, Deserialize)]
struct RootConfig {
    updates: Option<UpdatesConfig>,
}

#[derive(Debug, Deserialize)]
struct UpdatesConfig {
    mode: Option<UpdateMode>,
}

#[derive(Debug, Deserialize)]
struct InstallReceipt {
    schema_version: u8,
    kind: String,
    target: String,
    binary: PathBuf,
    install_id: String,
}

#[derive(Debug, Clone)]
enum InstallOwnership {
    Standalone(ManagedInstall),
    PackageManager { hint: &'static str },
    Unknown,
}

fn installation_ownership() -> InstallOwnership {
    let current = match env::current_exe().and_then(fs::canonicalize) {
        Ok(current) => current,
        Err(_) => return InstallOwnership::Unknown,
    };
    if let Some(hint) = package_manager_hint(&current) {
        return InstallOwnership::PackageManager { hint };
    }
    if let Some(paths) = UpdatePaths::discover()
        && let Ok(bytes) = fs::read(paths.receipt)
        && let Ok(receipt) = serde_json::from_slice::<InstallReceipt>(&bytes)
        && receipt.schema_version == 1
        && receipt.kind == "standalone"
        && receipt.target == BUILD_TARGET
        && !receipt.install_id.trim().is_empty()
        && fs::canonicalize(receipt.binary).ok().as_ref() == Some(&current)
    {
        return InstallOwnership::Standalone(ManagedInstall {
            binary: current,
            install_id: receipt.install_id,
        });
    }

    InstallOwnership::Unknown
}

fn package_manager_hint(current: &Path) -> Option<&'static str> {
    let path = current.to_string_lossy().to_ascii_lowercase();
    if path.contains("/.cargo/bin/") || path.contains("\\.cargo\\bin\\") {
        Some("cargo install crawlson --locked --force")
    } else if path.contains("/cellar/") || path.contains("/homebrew/") {
        Some("brew upgrade crawlson")
    } else if path.contains("/nix/store/") {
        Some("use the Nix configuration that installed Crawlson")
    } else {
        None
    }
}

fn configured_mode(ownership: InstallOwnership) -> UpdateMode {
    if let Ok(value) = env::var("CRAWLSON_UPDATE_POLICY") {
        return explicit_mode(&value);
    }
    if let Some(paths) = UpdatePaths::discover() {
        match fs::read_to_string(paths.config) {
            Ok(contents) => {
                return config_file_mode(&contents);
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return UpdateMode::Off,
            Err(_) => {}
        }
    }
    match ownership {
        InstallOwnership::Standalone(_) => UpdateMode::Auto,
        InstallOwnership::PackageManager { .. } | InstallOwnership::Unknown => UpdateMode::Notify,
    }
}

fn explicit_mode(value: &str) -> UpdateMode {
    parse_mode(value).unwrap_or(UpdateMode::Off)
}

fn config_file_mode(contents: &str) -> UpdateMode {
    toml::from_str::<RootConfig>(contents)
        .ok()
        .and_then(|config| config.updates)
        .and_then(|updates| updates.mode)
        .unwrap_or(UpdateMode::Off)
}

fn parse_mode(value: &str) -> Option<UpdateMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(UpdateMode::Auto),
        "notify" => Some(UpdateMode::Notify),
        "off" => Some(UpdateMode::Off),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PeriodicContext {
    ci: bool,
    no_update_check: bool,
    offline: bool,
    do_not_track: bool,
    auto_upgrade_disabled: bool,
}

impl PeriodicContext {
    fn from_env() -> Self {
        Self {
            ci: ci_value_is_enabled(env::var("CI").ok().as_deref()),
            no_update_check: env_truthy("CRAWLSON_NO_UPDATE_CHECK"),
            offline: env_truthy("CRAWLSON_OFFLINE"),
            do_not_track: env_truthy("DO_NOT_TRACK"),
            auto_upgrade_disabled: env::var("CRAWLSON_AUTO_UPGRADE")
                .is_ok_and(|value| value.trim() == "0"),
        }
    }
}

fn ci_value_is_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim().to_ascii_lowercase();
        !value.is_empty() && !matches!(value.as_str(), "0" | "false" | "no" | "off")
    })
}

fn periodic_allowed(context: PeriodicContext, mode: UpdateMode) -> bool {
    !context.ci
        && !context.no_update_check
        && !context.offline
        && !context.do_not_track
        && !context.auto_upgrade_disabled
        && mode != UpdateMode::Off
}

fn env_truthy(name: &str) -> bool {
    env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn read_state(path: &Path) -> Result<UpdateState, UpdateError> {
    let bytes = fs::read(path).map_err(state_error)?;
    let state: UpdateState = serde_json::from_slice(&bytes).map_err(state_error)?;
    if state.schema_version != 1 || state.install_id.is_empty() {
        return Err(UpdateError::State("unsupported update state".to_owned()));
    }
    Ok(state)
}

fn write_state(path: &Path, state: &UpdateState) -> Result<(), UpdateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(state_error)?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(state_error)?;
    serde_json::to_writer(&mut file, state).map_err(state_error)?;
    file.write_all(b"\n").map_err(state_error)?;
    file.commit().map_err(state_error)
}

fn validate_candidate_version(current: &Version, candidate: &Version) -> Result<(), String> {
    if !candidate.pre.is_empty() {
        return Err(format!(
            "refusing prerelease update candidate {candidate} on the stable channel"
        ));
    }
    if candidate == current {
        return Err(format!("Crawlson {current} is already up to date"));
    }
    if candidate < current {
        return Err(format!(
            "refusing to downgrade Crawlson from {current} to {candidate}"
        ));
    }
    Ok(())
}

fn auto_compatible(current: &Version, candidate: &Version) -> bool {
    if current.major == 0 {
        candidate.major == 0 && candidate.minor == current.minor
    } else {
        candidate.major == current.major
    }
}

fn success_delay(install_id: &str, bucket: u64) -> u64 {
    CHECK_SUCCESS_INTERVAL + jitter_hash(install_id, bucket) % (SUCCESS_JITTER_MAX + 1)
}

fn failure_delay(install_id: &str, bucket: u64) -> u64 {
    CHECK_FAILURE_INTERVAL + jitter_hash(install_id, bucket) % (FAILURE_JITTER_MAX + 1)
}

fn jitter_hash(install_id: &str, bucket: u64) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(install_id.as_bytes());
    hasher.update(bucket.to_le_bytes());
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().expect("digest has eight bytes"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn verified_candidate_from_manifest(
    release: &GithubRelease,
    manifest: &UpdateManifest,
    target: &str,
) -> Result<VerifiedCandidate, UpdateError> {
    if release.draft || release.prerelease || !release.immutable {
        return Err(UpdateError::InvalidMetadata(
            "the latest release is not a stable immutable release".to_owned(),
        ));
    }
    if manifest.schema_version != 1 {
        return Err(UpdateError::InvalidMetadata(format!(
            "unsupported manifest schema {}",
            manifest.schema_version
        )));
    }
    let tag_version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    if manifest.version.to_string() != tag_version || !manifest.version.pre.is_empty() {
        return Err(UpdateError::InvalidMetadata(
            "manifest version does not match the stable release tag".to_owned(),
        ));
    }
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.target == target)
        .ok_or_else(|| UpdateError::UnsupportedTarget(target.to_owned()))?;
    validate_sha256(&artifact.sha256)?;
    if artifact.size == 0 || artifact.size > MAX_BINARY_BYTES {
        return Err(UpdateError::InvalidMetadata(
            "manifest artifact size is outside the accepted range".to_owned(),
        ));
    }
    let release_asset = find_uploaded_asset(&release.assets, &artifact.name)?;
    if release_asset.size != artifact.size {
        return Err(UpdateError::InvalidMetadata(
            "manifest and release asset sizes disagree".to_owned(),
        ));
    }
    let expected_api_digest = format!("sha256:{}", artifact.sha256);
    if release_asset.digest.as_deref() != Some(expected_api_digest.as_str()) {
        return Err(UpdateError::InvalidMetadata(
            "manifest and GitHub asset digests disagree".to_owned(),
        ));
    }
    validate_download_url(&release_asset.browser_download_url)?;

    Ok(VerifiedCandidate {
        version: manifest.version.clone(),
        target: target.to_owned(),
        asset_name: artifact.name.clone(),
        size: artifact.size,
        sha256: artifact.sha256.clone(),
        release_url: release.html_url.clone(),
        download_url: release_asset.browser_download_url.clone(),
    })
}

fn find_uploaded_asset<'a>(
    assets: &'a [GithubAsset],
    name: &str,
) -> Result<&'a GithubAsset, UpdateError> {
    assets
        .iter()
        .find(|asset| asset.name == name && asset.state == "uploaded")
        .ok_or_else(|| UpdateError::InvalidMetadata(format!("missing release asset {name}")))
}

fn validate_download_url(url: &str) -> Result<(), UpdateError> {
    if url.starts_with(RELEASE_DOWNLOAD_PREFIX) {
        Ok(())
    } else {
        Err(UpdateError::InvalidMetadata(
            "release asset URL is outside the canonical Crawlson repository".to_owned(),
        ))
    }
}

fn validate_sha256(digest: &str) -> Result<(), UpdateError> {
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(UpdateError::InvalidMetadata(
            "artifact SHA-256 is malformed".to_owned(),
        ))
    }
}

fn verify_bytes_digest(
    bytes: &[u8],
    expected: Option<&str>,
    name: &str,
) -> Result<(), UpdateError> {
    let expected = expected.ok_or_else(|| {
        UpdateError::InvalidMetadata(format!("release asset {name} has no GitHub digest"))
    })?;
    let actual = format!("sha256:{}", hex_digest(Sha256::digest(bytes)));
    if actual == expected {
        Ok(())
    } else {
        Err(UpdateError::Verification(format!(
            "release asset {name} did not match its GitHub digest"
        )))
    }
}

fn verify_manifest_signature(
    public_key_text: &str,
    manifest: &[u8],
    signature_bytes: &[u8],
) -> Result<(), UpdateError> {
    let public_key = PublicKey::decode(public_key_text)
        .map_err(|error| UpdateError::InvalidSignature(error.to_string()))?;
    let signature_text = std::str::from_utf8(signature_bytes)
        .map_err(|error| UpdateError::InvalidSignature(error.to_string()))?;
    let signature = Signature::decode(signature_text)
        .map_err(|error| UpdateError::InvalidSignature(error.to_string()))?;
    public_key
        .verify(manifest, &signature, false)
        .map_err(|error| UpdateError::InvalidSignature(error.to_string()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| UpdateError::Download(error.to_string()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| UpdateError::Download(error.to_string()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), UpdateError> {
    Ok(())
}

#[cfg(unix)]
fn replace_executable(path: &Path) -> Result<(), UpdateError> {
    self_replace::self_replace(path).map_err(|error| UpdateError::Replacement(error.to_string()))
}

#[cfg(windows)]
fn replace_executable(_path: &Path) -> Result<(), UpdateError> {
    Err(UpdateError::Replacement(
        "direct self-upgrade is disabled on Windows until rollback is proven; use the installer"
            .to_owned(),
    ))
}

#[cfg(not(any(unix, windows)))]
fn replace_executable(_path: &Path) -> Result<(), UpdateError> {
    Err(UpdateError::Replacement(
        "direct self-upgrade is unsupported on this platform".to_owned(),
    ))
}

fn state_error(error: impl std::fmt::Display) -> UpdateError {
    UpdateError::State(error.to_string())
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    struct FakeBackend {
        candidate: Option<VerifiedCandidate>,
        checks: Cell<u32>,
        installs: Cell<u32>,
    }

    impl FakeBackend {
        fn new(candidate: Option<VerifiedCandidate>) -> Self {
            Self {
                candidate,
                checks: Cell::new(0),
                installs: Cell::new(0),
            }
        }
    }

    impl UpdateBackend for FakeBackend {
        fn check(&self, _target: &str) -> Result<Option<VerifiedCandidate>, UpdateError> {
            self.checks.set(self.checks.get() + 1);
            Ok(self.candidate.clone())
        }

        fn install(
            &self,
            _candidate: &VerifiedCandidate,
            _install: &ManagedInstall,
        ) -> Result<(), UpdateError> {
            self.installs.set(self.installs.get() + 1);
            Ok(())
        }
    }

    struct FailingBackend;

    impl UpdateBackend for FailingBackend {
        fn check(&self, _target: &str) -> Result<Option<VerifiedCandidate>, UpdateError> {
            Err(UpdateError::Request("fixture failure".to_owned()))
        }

        fn install(
            &self,
            _candidate: &VerifiedCandidate,
            _install: &ManagedInstall,
        ) -> Result<(), UpdateError> {
            panic!("a failing check must never install")
        }
    }

    #[test]
    fn version_policy_accepts_only_newer_stable_versions() {
        let current = Version::parse("0.1.0").unwrap();
        assert!(validate_candidate_version(&current, &Version::parse("0.1.1").unwrap()).is_ok());
        assert!(validate_candidate_version(&current, &Version::parse("0.1.0").unwrap()).is_err());
        assert!(validate_candidate_version(&current, &Version::parse("0.0.9").unwrap()).is_err());
        assert!(
            validate_candidate_version(&current, &Version::parse("0.2.0-alpha.1").unwrap())
                .is_err()
        );
    }

    #[test]
    fn automatic_updates_are_compatible_with_the_current_pre_one_series() {
        let current = Version::parse("0.1.2").unwrap();
        assert!(auto_compatible(&current, &Version::parse("0.1.3").unwrap()));
        assert!(!auto_compatible(
            &current,
            &Version::parse("0.2.0").unwrap()
        ));
    }

    #[test]
    fn cadence_jitter_stays_inside_the_documented_bounds() {
        let success = success_delay("install-a", 1);
        assert!(
            (CHECK_SUCCESS_INTERVAL..=CHECK_SUCCESS_INTERVAL + SUCCESS_JITTER_MAX)
                .contains(&success)
        );
        let failure = failure_delay("install-a", 1);
        assert!(
            (CHECK_FAILURE_INTERVAL..=CHECK_FAILURE_INTERVAL + FAILURE_JITTER_MAX)
                .contains(&failure)
        );
        assert_eq!(success, success_delay("install-a", 1));
    }

    #[test]
    fn parses_update_modes_strictly() {
        assert_eq!(parse_mode("auto"), Some(UpdateMode::Auto));
        assert_eq!(parse_mode("NOTIFY"), Some(UpdateMode::Notify));
        assert_eq!(parse_mode("off"), Some(UpdateMode::Off));
        assert_eq!(parse_mode("sometimes"), None);
        assert_eq!(explicit_mode("of"), UpdateMode::Off);
        assert_eq!(config_file_mode("not valid toml = ["), UpdateMode::Off);
        assert_eq!(
            config_file_mode("[updates]\nmode = 'notify'"),
            UpdateMode::Notify
        );
    }

    #[test]
    fn conventional_ci_values_disable_periodic_work() {
        assert!(ci_value_is_enabled(Some("buildkite")));
        assert!(ci_value_is_enabled(Some("1")));
        assert!(!ci_value_is_enabled(Some("false")));
        assert!(!ci_value_is_enabled(Some("0")));
        assert!(!ci_value_is_enabled(Some("")));
        assert!(!ci_value_is_enabled(None));
    }

    #[test]
    fn download_urls_are_scoped_to_the_canonical_repository() {
        assert!(
            validate_download_url(
                "https://github.com/jmitchel3/crawlson/releases/download/v0.1.1/crawlson"
            )
            .is_ok()
        );
        assert!(validate_download_url("https://example.com/crawlson").is_err());
    }

    #[test]
    fn signed_manifest_verification_rejects_tampering() {
        let public_key = "untrusted comment: minisign public key E7620F1842B4E81F\n\
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let signature = "untrusted comment: signature from minisign secret key\n\
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n\
trusted comment: timestamp:1556193335\tfile:test\n\
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";

        verify_manifest_signature(public_key, b"test", signature.as_bytes()).unwrap();
        assert!(verify_manifest_signature(public_key, b"Test", signature.as_bytes()).is_err());
    }

    #[test]
    fn check_only_never_invokes_the_installer() {
        let backend = FakeBackend::new(Some(candidate("0.4.1")));
        let result = run_manual_with_backend(
            ManualUpgradeOptions {
                check_only: true,
                offline: false,
                json: true,
            },
            &backend,
            InstallOwnership::Unknown,
        );

        assert_eq!(result.exit_code, 0);
        assert_eq!(backend.checks.get(), 1);
        assert_eq!(backend.installs.get(), 0);
        let report: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(report["status"], "update_available");
    }

    #[test]
    fn managed_manual_upgrade_invokes_the_injected_installer() {
        let backend = FakeBackend::new(Some(candidate("0.4.1")));
        let result = run_manual_with_backend(
            ManualUpgradeOptions {
                check_only: false,
                offline: false,
                json: true,
            },
            &backend,
            InstallOwnership::Standalone(ManagedInstall {
                binary: PathBuf::from("crawlson"),
                install_id: "fixture-install".to_owned(),
            }),
        );

        assert_eq!(result.exit_code, 0);
        assert_eq!(backend.installs.get(), 1);
        let report: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(report["status"], "upgraded");
    }

    #[test]
    fn rendered_manual_policy_rejects_downgrades_and_prereleases() {
        for version in ["0.3.9", "0.4.1-alpha.1"] {
            let backend = FakeBackend::new(Some(candidate(version)));
            let result = run_manual_with_backend(
                ManualUpgradeOptions {
                    check_only: false,
                    offline: false,
                    json: true,
                },
                &backend,
                InstallOwnership::Unknown,
            );
            assert_eq!(result.exit_code, 1, "{version}");
            assert_eq!(backend.installs.get(), 0, "{version}");
            let report: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
            assert_eq!(report["status"], "blocked", "{version}");
        }
    }

    #[test]
    fn package_manager_paths_take_precedence_over_receipts() {
        assert_eq!(
            package_manager_hint(Path::new("/tmp/.cargo/bin/crawlson")),
            Some("cargo install crawlson --locked --force")
        );
        assert_eq!(
            package_manager_hint(Path::new("/opt/homebrew/Cellar/crawlson/0.1/bin/crawlson")),
            Some("brew upgrade crawlson")
        );
        assert!(package_manager_hint(Path::new("/opt/crawlson/bin/crawlson")).is_none());
    }

    #[test]
    fn unknown_installations_fail_closed_before_replacement() {
        let backend = FakeBackend::new(Some(candidate("0.1.1")));
        let result = run_manual_with_backend(
            ManualUpgradeOptions {
                check_only: false,
                offline: false,
                json: true,
            },
            &backend,
            InstallOwnership::Unknown,
        );

        assert_eq!(result.exit_code, 1);
        assert_eq!(backend.installs.get(), 0);
        let report: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(report["status"], "blocked");
    }

    #[test]
    fn periodic_worker_reserves_the_next_check_before_returning() {
        let directory = tempfile::tempdir().unwrap();
        let paths = UpdatePaths {
            config: directory.path().join("config.toml"),
            receipt: directory.path().join("install.json"),
            state: directory.path().join("state.json"),
            lock: directory.path().join("state.lock"),
        };
        let backend = FakeBackend::new(None);

        periodic_worker(&paths, 1_000_000, &backend).unwrap();
        let state = read_state(&paths.state).unwrap();
        assert_eq!(backend.checks.get(), 1);
        assert_eq!(state.last_attempt_at, Some(1_000_000));
        assert!(state.next_check_at.unwrap() > 1_000_000);

        periodic_worker(&paths, 1_000_001, &backend).unwrap();
        assert_eq!(backend.checks.get(), 1);
    }

    #[test]
    fn every_periodic_opt_out_disables_background_work() {
        assert!(periodic_allowed(
            PeriodicContext::default(),
            UpdateMode::Auto
        ));
        for context in [
            PeriodicContext {
                ci: true,
                ..PeriodicContext::default()
            },
            PeriodicContext {
                no_update_check: true,
                ..PeriodicContext::default()
            },
            PeriodicContext {
                offline: true,
                ..PeriodicContext::default()
            },
            PeriodicContext {
                do_not_track: true,
                ..PeriodicContext::default()
            },
            PeriodicContext {
                auto_upgrade_disabled: true,
                ..PeriodicContext::default()
            },
        ] {
            assert!(!periodic_allowed(context, UpdateMode::Auto));
        }
        assert!(!periodic_allowed(
            PeriodicContext::default(),
            UpdateMode::Off
        ));
    }

    #[test]
    fn worker_failure_has_empty_output_and_a_success_exit() {
        let directory = tempfile::tempdir().unwrap();
        let paths = UpdatePaths {
            config: directory.path().join("config.toml"),
            receipt: directory.path().join("install.json"),
            state: directory.path().join("state.json"),
            lock: directory.path().join("state.lock"),
        };

        let result = periodic_worker_command(&paths, 1_000_000, &FailingBackend);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        let state = read_state(&paths.state).unwrap();
        assert_eq!(state.failure_count, 1);
        let next = state.next_check_at.unwrap();
        assert!(
            (1_000_000 + CHECK_FAILURE_INTERVAL
                ..=1_000_000 + CHECK_FAILURE_INTERVAL + FAILURE_JITTER_MAX)
                .contains(&next)
        );
    }

    #[test]
    fn manifest_and_release_metadata_must_agree_exactly() {
        let (mut release, manifest) = release_and_manifest();
        let candidate =
            verified_candidate_from_manifest(&release, &manifest, "test-target").unwrap();
        assert_eq!(candidate.version, Version::parse("0.1.1").unwrap());

        release.immutable = false;
        assert!(verified_candidate_from_manifest(&release, &manifest, "test-target").is_err());
        release.immutable = true;
        release.assets[0].digest = Some(format!("sha256:{}", "f".repeat(64)));
        assert!(verified_candidate_from_manifest(&release, &manifest, "test-target").is_err());
        assert!(verified_candidate_from_manifest(&release, &manifest, "wrong-target").is_err());
    }

    fn candidate(version: &str) -> VerifiedCandidate {
        VerifiedCandidate {
            version: Version::parse(version).unwrap(),
            target: BUILD_TARGET.to_owned(),
            asset_name: "crawlson-test".to_owned(),
            size: 1,
            sha256: "0".repeat(64),
            release_url: "https://github.com/jmitchel3/crawlson/releases/tag/test".to_owned(),
            download_url:
                "https://github.com/jmitchel3/crawlson/releases/download/test/crawlson-test"
                    .to_owned(),
        }
    }

    fn release_and_manifest() -> (GithubRelease, UpdateManifest) {
        let digest = "0".repeat(64);
        (
            GithubRelease {
                tag_name: "v0.1.1".to_owned(),
                html_url: "https://github.com/jmitchel3/crawlson/releases/tag/v0.1.1"
                    .to_owned(),
                draft: false,
                prerelease: false,
                immutable: true,
                assets: vec![GithubAsset {
                    name: "crawlson-test".to_owned(),
                    state: "uploaded".to_owned(),
                    size: 10,
                    digest: Some(format!("sha256:{digest}")),
                    browser_download_url:
                        "https://github.com/jmitchel3/crawlson/releases/download/v0.1.1/crawlson-test"
                            .to_owned(),
                }],
            },
            UpdateManifest {
                schema_version: 1,
                version: Version::parse("0.1.1").unwrap(),
                artifacts: vec![ManifestArtifact {
                    target: "test-target".to_owned(),
                    name: "crawlson-test".to_owned(),
                    size: 10,
                    sha256: digest,
                }],
            },
        )
    }
}
