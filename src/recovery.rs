//! Durable, non-secret recovery barriers for mutating journeys.
//!
//! The origin-keyed journal is the authority. Its presence prevents another
//! mutation for that exact origin until fixture cleanup has been independently
//! verified. A second copy in the run directory makes the incomplete cleanup
//! visible with the rest of the run evidence.
//!
//! Creating or removing two files on potentially different filesystems cannot
//! be one filesystem transaction. `begin` therefore creates the authoritative
//! origin barrier first, and `complete_verified` removes it last. An
//! interruption can leave an extra barrier or run copy, but never deliberately
//! removes the barrier before the run copy. Callers must treat every error as a
//! blocked mutation and must never call `complete_verified` until cleanup has
//! been verified from the fixture provider.

use std::collections::HashSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::journey::Origin;

pub const RECOVERY_SCHEMA_VERSION: u8 = 1;
pub const RECOVERY_DIRECTORY: &str = ".crawlson-recovery";
pub const RUN_RECOVERY_FILE: &str = "recovery-required.json";

const MAX_RECOVERY_BYTES: u64 = 64 * 1024;
const MAX_CLEANUP_STEPS: usize = 256;
const MAX_ID_BYTES: usize = 96;
const MAX_RUN_ID_BYTES: usize = 128;
const MAX_RUN_DIRECTORY_BYTES: usize = 255;

/// The complete public information needed to identify and finish cleanup.
///
/// Deliberately do not add authentication state, fixture values, fixture
/// tokens, request URLs, provider stdout/stderr, or provider-specific payloads
/// to this type. The exact origin is the most specific target location that a
/// recovery record may contain.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub schema_version: u8,
    pub journey_id: String,
    pub revision: u32,
    pub source_sha256: String,
    pub target_origin: String,
    pub run_id: String,
    /// A single directory name, never a filesystem path.
    pub run_directory: String,
    pub cleanup_step_ids: Vec<String>,
    pub created_at_unix_ms: u64,
}

impl RecoveryRecord {
    pub fn validate(&self) -> Result<(), RecoveryError> {
        if self.schema_version != RECOVERY_SCHEMA_VERSION
            || !valid_identifier(&self.journey_id, MAX_ID_BYTES)
            || self.revision == 0
            || !is_sha256(&self.source_sha256)
            || !valid_run_identifier(&self.run_id, MAX_RUN_ID_BYTES)
            || !valid_directory_label(&self.run_directory)
            || self.run_directory != format!("crawlson-run-{}", self.run_id)
            || self.created_at_unix_ms == 0
            || self.cleanup_step_ids.is_empty()
            || self.cleanup_step_ids.len() > MAX_CLEANUP_STEPS
        {
            return Err(RecoveryError::InvalidRecord);
        }

        let parsed =
            Origin::parse(&self.target_origin).map_err(|_| RecoveryError::InvalidRecord)?;
        if parsed.to_string() != self.target_origin {
            return Err(RecoveryError::InvalidRecord);
        }

        let mut unique = HashSet::with_capacity(self.cleanup_step_ids.len());
        if self
            .cleanup_step_ids
            .iter()
            .any(|step| !valid_identifier(step, MAX_ID_BYTES) || !unique.insert(step.as_str()))
        {
            return Err(RecoveryError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    #[error("the recovery store is not a safe local directory")]
    InvalidStore,
    #[error("the recovery record is invalid")]
    InvalidRecord,
    #[error("the recovery journal is invalid or was modified")]
    InvalidJournal,
    #[error("cleanup recovery is already pending for this exact origin")]
    Pending,
    #[error("the recovery journal could not be created")]
    BeginFailed,
    #[error("the origin barrier was created but the run recovery copy could not be created")]
    PartialBegin,
    #[error("verified recovery could not be completed safely")]
    CompleteFailed,
}

impl RecoveryError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidStore => "recovery_store_invalid",
            Self::InvalidRecord => "recovery_record_invalid",
            Self::InvalidJournal => "recovery_journal_invalid",
            Self::Pending => "recovery_pending",
            Self::BeginFailed => "recovery_begin_failed",
            Self::PartialBegin => "recovery_begin_partial",
            Self::CompleteFailed => "recovery_complete_failed",
        }
    }
}

#[derive(Debug)]
pub struct RecoveryStore {
    output_base: PathBuf,
    root: PathBuf,
    root_parent: PathBuf,
    root_identity: DirectoryIdentity,
}

impl RecoveryStore {
    /// Opens the recovery store below `output_base`, creating the two
    /// directories when needed. The retained base path is canonical, and the
    /// store itself must always remain a real, non-symlink directory.
    pub fn new(output_base: &Path) -> Result<Self, RecoveryError> {
        Self::at_root(output_base, output_base)
    }

    /// Opens the process-independent recovery authority in Crawlson's user
    /// state directory while retaining `output_base` only for the run copy.
    /// `CRAWLSON_HOME` follows the same explicit override used by update and
    /// installation state.
    pub fn global(output_base: &Path) -> Result<Self, RecoveryError> {
        let state_root = if let Some(root) = env::var_os("CRAWLSON_HOME") {
            let root = PathBuf::from(root);
            if root.as_os_str().is_empty() {
                return Err(RecoveryError::InvalidStore);
            }
            root
        } else {
            let dirs = ProjectDirs::from("org", "crawlson", "crawlson")
                .ok_or(RecoveryError::InvalidStore)?;
            dirs.state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .to_path_buf()
        };
        Self::at_root(output_base, &state_root)
    }

    fn at_root(output_base: &Path, root_parent: &Path) -> Result<Self, RecoveryError> {
        fs::create_dir_all(output_base).map_err(|_| RecoveryError::InvalidStore)?;
        let supplied_metadata =
            fs::symlink_metadata(output_base).map_err(|_| RecoveryError::InvalidStore)?;
        if supplied_metadata.file_type().is_symlink() || !supplied_metadata.is_dir() {
            return Err(RecoveryError::InvalidStore);
        }
        let output_base = output_base
            .canonicalize()
            .map_err(|_| RecoveryError::InvalidStore)?;
        fs::create_dir_all(root_parent).map_err(|_| RecoveryError::InvalidStore)?;
        let root_parent = root_parent
            .canonicalize()
            .map_err(|_| RecoveryError::InvalidStore)?;
        let root = root_parent.join(RECOVERY_DIRECTORY);
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(RecoveryError::InvalidStore);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_private_directory(&root).map_err(|_| RecoveryError::InvalidStore)?;
            }
            Err(_) => return Err(RecoveryError::InvalidStore),
        }
        let root = root
            .canonicalize()
            .map_err(|_| RecoveryError::InvalidStore)?;
        let root_identity = directory_identity(&root)?;
        Ok(Self {
            output_base,
            root,
            root_parent,
            root_identity,
        })
    }

    /// Returns a pending record for `origin`. Any malformed, oversized,
    /// linked, replaced, or mismatched journal is an error rather than an
    /// absent journal.
    pub fn check_pending(&self, origin: &Origin) -> Result<Option<RecoveryRecord>, RecoveryError> {
        self.validate_store()?;
        let origin = origin.to_string();
        let path = self.journal_path(&origin);
        let stored = match read_journal(&path) {
            Ok(stored) => stored,
            Err(ReadJournalError::Missing) => return Ok(None),
            Err(ReadJournalError::Invalid) => return Err(RecoveryError::InvalidJournal),
        };
        if stored.record.target_origin != origin {
            return Err(RecoveryError::InvalidJournal);
        }
        Ok(Some(stored.record))
    }

    /// Creates the authoritative origin barrier and its run-directory copy.
    ///
    /// A `PartialBegin` deliberately leaves the origin barrier in place. This
    /// blocks subsequent mutations after disk errors or interruption instead
    /// of guessing whether fixture setup or cleanup occurred.
    pub fn begin(
        &self,
        record: RecoveryRecord,
        run_root: &Path,
    ) -> Result<ActiveRecovery, RecoveryError> {
        self.validate_store()?;
        record.validate()?;
        let origin =
            Origin::parse(&record.target_origin).map_err(|_| RecoveryError::InvalidRecord)?;
        let run_root = self.validate_run_root(run_root, &record.run_directory)?;
        let run_root_identity = directory_identity(&run_root)?;
        let central_path = self.journal_path(&origin.to_string());
        let run_path = run_root.join(RUN_RECOVERY_FILE);

        match read_journal(&central_path) {
            Ok(_) => return Err(RecoveryError::Pending),
            Err(ReadJournalError::Invalid) => return Err(RecoveryError::InvalidJournal),
            Err(ReadJournalError::Missing) => {}
        }
        match fs::symlink_metadata(&run_path) {
            Ok(_) => return Err(RecoveryError::Pending),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RecoveryError::BeginFailed),
        }

        let bytes = encoded_journal(&record)?;
        match write_new_journal(&central_path, &bytes) {
            Ok(()) => {}
            Err(WriteJournalError::AlreadyExists) => {
                return match read_journal(&central_path) {
                    Ok(_) => Err(RecoveryError::Pending),
                    Err(_) => Err(RecoveryError::InvalidJournal),
                };
            }
            Err(WriteJournalError::CreatedIncomplete) => {
                return Err(RecoveryError::PartialBegin);
            }
            Err(WriteJournalError::Failed) => return Err(RecoveryError::BeginFailed),
        }
        let recovery_lock = lock_journal(&central_path).map_err(|_| RecoveryError::PartialBegin)?;
        if write_new_journal(&run_path, &bytes).is_err() {
            return Err(RecoveryError::PartialBegin);
        }

        Ok(ActiveRecovery {
            root: self.root.clone(),
            root_identity: self.root_identity,
            run_root,
            run_root_identity,
            central_path,
            run_path,
            record,
            journal_sha256: hex_digest(&bytes),
            recovery_lock,
        })
    }

    /// Rebinds an already-pending exact-origin authority to a new recovery
    /// run. The authoritative record must match byte-for-byte; only the new
    /// run copy is created. Verified cleanup may then remove the new copy and
    /// the global authority, while the original run marker remains historical
    /// evidence of that run's incomplete cleanup.
    pub fn resume(
        &self,
        record: RecoveryRecord,
        run_root: &Path,
    ) -> Result<ActiveRecovery, RecoveryError> {
        self.validate_store()?;
        record.validate()?;
        let run_root = self.validate_run_root(
            run_root,
            run_root
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or(RecoveryError::InvalidStore)?,
        )?;
        let run_root_identity = directory_identity(&run_root)?;
        let central_path = self.journal_path(&record.target_origin);
        let recovery_lock = lock_journal(&central_path)?;
        let stored = read_journal(&central_path).map_err(|error| match error {
            ReadJournalError::Missing => RecoveryError::Pending,
            ReadJournalError::Invalid => RecoveryError::InvalidJournal,
        })?;
        if stored.record != record {
            return Err(RecoveryError::InvalidJournal);
        }
        let run_path = run_root.join(RUN_RECOVERY_FILE);
        match fs::symlink_metadata(&run_path) {
            Ok(_) => return Err(RecoveryError::Pending),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RecoveryError::PartialBegin),
        }
        let bytes = encoded_journal(&record)?;
        if write_new_journal(&run_path, &bytes).is_err() {
            return Err(RecoveryError::PartialBegin);
        }
        Ok(ActiveRecovery {
            root: self.root.clone(),
            root_identity: self.root_identity,
            run_root,
            run_root_identity,
            central_path,
            run_path,
            record,
            journal_sha256: hex_digest(&bytes),
            recovery_lock,
        })
    }

    fn journal_path(&self, exact_origin: &str) -> PathBuf {
        self.root
            .join(format!("{}.json", hex_digest(exact_origin.as_bytes())))
    }

    fn validate_store(&self) -> Result<(), RecoveryError> {
        if directory_identity(&self.root)? != self.root_identity
            || self.root.parent() != Some(self.root_parent.as_path())
        {
            return Err(RecoveryError::InvalidStore);
        }
        Ok(())
    }

    fn validate_run_root(
        &self,
        run_root: &Path,
        expected_label: &str,
    ) -> Result<PathBuf, RecoveryError> {
        let supplied = fs::symlink_metadata(run_root).map_err(|_| RecoveryError::InvalidStore)?;
        if supplied.file_type().is_symlink() || !supplied.is_dir() {
            return Err(RecoveryError::InvalidStore);
        }
        let canonical = run_root
            .canonicalize()
            .map_err(|_| RecoveryError::InvalidStore)?;
        if canonical.parent() != Some(self.output_base.as_path())
            || canonical == self.root
            || canonical.file_name().and_then(|name| name.to_str()) != Some(expected_label)
        {
            return Err(RecoveryError::InvalidStore);
        }
        directory_identity(&canonical)?;
        Ok(canonical)
    }
}

/// A live recovery barrier. Dropping this value intentionally does nothing.
/// Only verified fixture cleanup may call `complete_verified`.
#[derive(Debug)]
#[must_use = "dropping an active recovery keeps the mutation recovery barrier pending"]
pub struct ActiveRecovery {
    root: PathBuf,
    root_identity: DirectoryIdentity,
    run_root: PathBuf,
    run_root_identity: DirectoryIdentity,
    central_path: PathBuf,
    run_path: PathBuf,
    record: RecoveryRecord,
    journal_sha256: String,
    /// Holds an operating-system exclusive lock on the origin authority for
    /// the complete main/cleanup or recovery-only lifetime.
    recovery_lock: fs::File,
}

impl ActiveRecovery {
    pub fn record(&self) -> &RecoveryRecord {
        &self.record
    }

    /// Removes the run marker first and the authoritative origin barrier last.
    /// Each unlink is atomic. Keeping the authority until last makes process
    /// interruption conservative despite the lack of a cross-directory
    /// transaction.
    pub fn complete_verified(self) -> Result<(), RecoveryError> {
        let _lock_is_held = &self.recovery_lock;
        if directory_identity(&self.root).map_err(|_| RecoveryError::CompleteFailed)?
            != self.root_identity
        {
            return Err(RecoveryError::CompleteFailed);
        }
        if directory_identity(&self.run_root).map_err(|_| RecoveryError::CompleteFailed)?
            != self.run_root_identity
            || self.run_path.parent() != Some(self.run_root.as_path())
            || self.central_path.parent() != Some(self.root.as_path())
        {
            return Err(RecoveryError::CompleteFailed);
        }

        self.verify_copy(&self.central_path)?;
        self.verify_copy(&self.run_path)?;
        remove_verified_file(&self.run_path).map_err(|_| RecoveryError::CompleteFailed)?;
        sync_directory(&self.run_root).map_err(|_| RecoveryError::CompleteFailed)?;

        // Recheck the authoritative file after removing the run copy. If it
        // changed concurrently, leave it in place and continue to fail closed.
        self.verify_copy(&self.central_path)?;
        remove_verified_file(&self.central_path).map_err(|_| RecoveryError::CompleteFailed)?;
        sync_directory(&self.root).map_err(|_| RecoveryError::CompleteFailed)?;

        Ok(())
    }

    fn verify_copy(&self, path: &Path) -> Result<(), RecoveryError> {
        let stored = read_journal(path).map_err(|_| RecoveryError::CompleteFailed)?;
        if stored.record != self.record || stored.bytes_sha256 != self.journal_sha256 {
            return Err(RecoveryError::CompleteFailed);
        }
        Ok(())
    }
}

fn lock_journal(path: &Path) -> Result<fs::File, RecoveryError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RecoveryError::InvalidJournal)?;
    validate_file_metadata(&metadata).map_err(|_| RecoveryError::InvalidJournal)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    add_no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|_| RecoveryError::InvalidJournal)?;
    validate_opened_file(&file).map_err(|_| RecoveryError::InvalidJournal)?;
    if !same_file(
        &metadata,
        &file.metadata().map_err(|_| RecoveryError::InvalidJournal)?,
    ) || !opened_path_still_matches(path, &file)
    {
        return Err(RecoveryError::InvalidJournal);
    }
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err(RecoveryError::Pending),
        Err(fs::TryLockError::Error(_)) => Err(RecoveryError::InvalidJournal),
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredDocument {
    record: RecoveryRecord,
    record_sha256: String,
}

struct StoredJournal {
    record: RecoveryRecord,
    bytes_sha256: String,
}

fn encoded_journal(record: &RecoveryRecord) -> Result<Vec<u8>, RecoveryError> {
    record.validate()?;
    let record_bytes = serde_json::to_vec(record).map_err(|_| RecoveryError::InvalidRecord)?;
    let document = StoredDocument {
        record: record.clone(),
        record_sha256: hex_digest(&record_bytes),
    };
    let mut bytes =
        serde_json::to_vec_pretty(&document).map_err(|_| RecoveryError::InvalidRecord)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_RECOVERY_BYTES {
        return Err(RecoveryError::InvalidRecord);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadJournalError {
    Missing,
    Invalid,
}

fn read_journal(path: &Path) -> Result<StoredJournal, ReadJournalError> {
    let path_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ReadJournalError::Missing);
        }
        Err(_) => return Err(ReadJournalError::Invalid),
    };
    validate_file_metadata(&path_metadata).map_err(|_| ReadJournalError::Invalid)?;

    let mut options = OpenOptions::new();
    options.read(true);
    add_no_follow(&mut options);
    let mut file = options.open(path).map_err(|_| ReadJournalError::Invalid)?;
    let opened_metadata = file.metadata().map_err(|_| ReadJournalError::Invalid)?;
    validate_file_metadata(&opened_metadata).map_err(|_| ReadJournalError::Invalid)?;
    validate_opened_file(&file).map_err(|_| ReadJournalError::Invalid)?;
    if !same_file(&path_metadata, &opened_metadata) || !opened_path_still_matches(path, &file) {
        return Err(ReadJournalError::Invalid);
    }

    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    (&mut file)
        .take(MAX_RECOVERY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadJournalError::Invalid)?;
    let after_metadata = file.metadata().map_err(|_| ReadJournalError::Invalid)?;
    let after_path_metadata = fs::symlink_metadata(path).map_err(|_| ReadJournalError::Invalid)?;
    if bytes.len() as u64 != opened_metadata.len()
        || !same_file(&opened_metadata, &after_metadata)
        || !same_file(&opened_metadata, &after_path_metadata)
        || !opened_path_still_matches(path, &file)
        || after_metadata.modified().ok() != opened_metadata.modified().ok()
    {
        return Err(ReadJournalError::Invalid);
    }

    let document: StoredDocument =
        serde_json::from_slice(&bytes).map_err(|_| ReadJournalError::Invalid)?;
    document
        .record
        .validate()
        .map_err(|_| ReadJournalError::Invalid)?;
    let record_bytes =
        serde_json::to_vec(&document.record).map_err(|_| ReadJournalError::Invalid)?;
    if !is_sha256(&document.record_sha256) || document.record_sha256 != hex_digest(&record_bytes) {
        return Err(ReadJournalError::Invalid);
    }
    Ok(StoredJournal {
        record: document.record,
        bytes_sha256: hex_digest(&bytes),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteJournalError {
    AlreadyExists,
    CreatedIncomplete,
    Failed,
}

fn write_new_journal(path: &Path, bytes: &[u8]) -> Result<(), WriteJournalError> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_RECOVERY_BYTES {
        return Err(WriteJournalError::Failed);
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_new_journal(&mut options);
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(WriteJournalError::AlreadyExists);
        }
        Err(_) => return Err(WriteJournalError::Failed),
    };
    file.write_all(bytes)
        .map_err(|_| WriteJournalError::CreatedIncomplete)?;
    file.sync_all()
        .map_err(|_| WriteJournalError::CreatedIncomplete)?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| WriteJournalError::CreatedIncomplete)?;
    let path_metadata =
        fs::symlink_metadata(path).map_err(|_| WriteJournalError::CreatedIncomplete)?;
    if opened_metadata.len() != bytes.len() as u64
        || !same_file(&opened_metadata, &path_metadata)
        || validate_file_metadata(&opened_metadata).is_err()
        || validate_opened_file(&file).is_err()
        || !opened_path_still_matches(path, &file)
    {
        return Err(WriteJournalError::CreatedIncomplete);
    }
    if let Some(parent) = path.parent() {
        sync_directory(parent).map_err(|_| WriteJournalError::CreatedIncomplete)?;
    }
    Ok(())
}

fn remove_verified_file(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    validate_file_metadata(&metadata)
        .map_err(|_| std::io::Error::other("unsafe recovery journal"))?;
    let mut options = OpenOptions::new();
    options.read(true);
    add_no_follow(&mut options);
    let file = options.open(path)?;
    validate_opened_file(&file).map_err(|_| std::io::Error::other("unsafe recovery journal"))?;
    if !opened_path_still_matches(path, &file) {
        return Err(std::io::Error::other("recovery journal was replaced"));
    }
    fs::remove_file(path)
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn valid_directory_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RUN_DIRECTORY_BYTES
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && valid_run_identifier(value, MAX_RUN_DIRECTORY_BYTES)
}

fn valid_run_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirectoryIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    creation_time: u64,
}

fn directory_identity(path: &Path) -> Result<DirectoryIdentity, RecoveryError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RecoveryError::InvalidStore)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RecoveryError::InvalidStore);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o022 != 0 {
            return Err(RecoveryError::InvalidStore);
        }
        Ok(DirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(RecoveryError::InvalidStore);
        }
        Ok(DirectoryIdentity {
            creation_time: metadata.creation_time(),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        Ok(DirectoryIdentity {})
    }
}

fn validate_file_metadata(metadata: &fs::Metadata) -> Result<(), ()> {
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_RECOVERY_BYTES
    {
        return Err(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o022 != 0 {
            return Err(());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.nlink() == 1
        && right.nlink() == 1
}

#[cfg(windows)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
        && left.file_size() == right.file_size()
        && left.file_attributes() == right.file_attributes()
}

#[cfg(not(any(unix, windows)))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.is_file()
        && right.is_file()
}

#[cfg(unix)]
fn validate_opened_file(file: &fs::File) -> Result<(), ()> {
    file.metadata()
        .map_err(|_| ())
        .and_then(|metadata| validate_file_metadata(&metadata))
}

#[cfg(windows)]
fn validate_opened_file(file: &fs::File) -> Result<(), ()> {
    if windows_file_identity(file)?.number_of_links != 1 {
        return Err(());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_opened_file(_file: &fs::File) -> Result<(), ()> {
    Ok(())
}

#[cfg(unix)]
fn opened_path_still_matches(path: &Path, file: &fs::File) -> bool {
    file.metadata()
        .ok()
        .zip(fs::symlink_metadata(path).ok())
        .is_some_and(|(opened, current)| {
            validate_file_metadata(&opened).is_ok()
                && validate_file_metadata(&current).is_ok()
                && same_file(&opened, &current)
        })
}

#[cfg(windows)]
fn opened_path_still_matches(path: &Path, file: &fs::File) -> bool {
    let mut options = OpenOptions::new();
    options.read(true);
    add_no_follow(&mut options);
    let Ok(current) = options.open(path) else {
        return false;
    };
    windows_file_identity(file)
        .ok()
        .zip(windows_file_identity(&current).ok())
        .is_some_and(|(opened, current)| opened == current && opened.number_of_links == 1)
}

#[cfg(not(any(unix, windows)))]
fn opened_path_still_matches(path: &Path, file: &fs::File) -> bool {
    file.metadata()
        .ok()
        .zip(fs::symlink_metadata(path).ok())
        .is_some_and(|(opened, current)| same_file(&opened, &current))
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct WindowsFileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct WindowsByHandleFileInformation {
    file_attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsFileIdentity {
    volume_serial_number: u32,
    file_index: u64,
    number_of_links: u32,
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> Result<WindowsFileIdentity, ()> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetFileInformationByHandle"]
        fn get_file_information_by_handle(
            file: *mut std::ffi::c_void,
            information: *mut WindowsByHandleFileInformation,
        ) -> i32;
    }

    let mut information = MaybeUninit::<WindowsByHandleFileInformation>::uninit();
    // SAFETY: the handle belongs to a live `File`, and Windows initializes the
    // entire fixed-layout output structure when the call succeeds.
    let succeeded = unsafe {
        get_file_information_by_handle(file.as_raw_handle().cast(), information.as_mut_ptr())
    };
    if succeeded == 0 {
        return Err(());
    }
    // SAFETY: a nonzero result from GetFileInformationByHandle initialized the
    // output structure.
    let information = unsafe { information.assume_init() };
    Ok(WindowsFileIdentity {
        volume_serial_number: information.volume_serial_number,
        file_index: (u64::from(information.file_index_high) << 32)
            | u64::from(information.file_index_low),
        number_of_links: information.number_of_links,
    })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn add_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    const O_NOFOLLOW: i32 = 0x0002_0000;
    options.custom_flags(O_NOFOLLOW);
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn add_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    const O_NOFOLLOW: i32 = 0x0000_0100;
    options.custom_flags(O_NOFOLLOW);
}

#[cfg(windows)]
fn add_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
fn add_no_follow(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn configure_new_journal(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(windows)]
fn configure_new_journal(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    // Windows has no dependable POSIX-style parent-directory fsync. Request
    // write-through when creating each journal so the file contents and the
    // metadata produced by the write reach storage before `begin` succeeds.
    // `sync_all` below remains required and reports any flush failure.
    const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
    options.custom_flags(FILE_FLAG_WRITE_THROUGH);
}

#[cfg(not(any(unix, windows)))]
fn configure_new_journal(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    fs::File::open(path).and_then(|directory| directory.sync_all())
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    // `File::sync_all` delegates to FlushFileBuffers, which requires a
    // GENERIC_WRITE file handle and is not a supported parent-directory fsync
    // primitive. Journal creation is instead opened with
    // FILE_FLAG_WRITE_THROUGH and followed by a file-level `sync_all`.
    // After verified cleanup, a namespace deletion lost to a crash can only
    // restore an extra barrier, which remains the fail-closed outcome.
    //
    // Still validate the path at each durability boundary so replacement by a
    // reparse point or non-directory fails closed.
    let metadata = fs::symlink_metadata(path)?;
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(std::io::Error::other("unsafe recovery directory"));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        tempfile::TempDir,
        RecoveryStore,
        PathBuf,
        RecoveryRecord,
        Origin,
    ) {
        let output = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(output.path()).unwrap();
        let run_root = output.path().join("crawlson-run-abc123");
        fs::create_dir(&run_root).unwrap();
        let origin = Origin::parse("http://127.0.0.1:4173").unwrap();
        let record = RecoveryRecord {
            schema_version: RECOVERY_SCHEMA_VERSION,
            journey_id: "demo.mutate-item".to_owned(),
            revision: 1,
            source_sha256: "a".repeat(64),
            target_origin: origin.to_string(),
            run_id: "abc123".to_owned(),
            run_directory: "crawlson-run-abc123".to_owned(),
            cleanup_step_ids: vec!["delete-item".to_owned(), "verify-empty".to_owned()],
            created_at_unix_ms: 1_700_000_000_000,
        };
        (output, store, run_root, record, origin)
    }

    #[test]
    fn begins_blocks_and_completes_only_explicitly() {
        let (_output, store, run_root, record, origin) = fixture();
        let active = store.begin(record.clone(), &run_root).unwrap();

        assert_eq!(store.check_pending(&origin).unwrap(), Some(record.clone()));
        assert!(run_root.join(RUN_RECOVERY_FILE).is_file());
        assert_eq!(
            store.begin(record, &run_root).unwrap_err(),
            RecoveryError::Pending
        );

        active.complete_verified().unwrap();
        assert_eq!(store.check_pending(&origin).unwrap(), None);
        assert!(!run_root.join(RUN_RECOVERY_FILE).exists());
    }

    #[test]
    fn one_global_authority_blocks_the_same_origin_across_output_bases() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let first_output = root.path().join("first-output");
        let second_output = root.path().join("second-output");
        let first = RecoveryStore::at_root(&first_output, &state).unwrap();
        let run_root = first_output.join("crawlson-run-abc123");
        fs::create_dir(&run_root).unwrap();
        let origin = Origin::parse("http://127.0.0.1:4173").unwrap();
        let record = RecoveryRecord {
            schema_version: RECOVERY_SCHEMA_VERSION,
            journey_id: "demo.mutate-item".to_owned(),
            revision: 1,
            source_sha256: "a".repeat(64),
            target_origin: origin.to_string(),
            run_id: "abc123".to_owned(),
            run_directory: "crawlson-run-abc123".to_owned(),
            cleanup_step_ids: vec!["delete-item".to_owned()],
            created_at_unix_ms: 1_700_000_000_000,
        };
        let active = first.begin(record.clone(), &run_root).unwrap();

        let second = RecoveryStore::at_root(&second_output, &state).unwrap();
        assert_eq!(second.check_pending(&origin).unwrap(), Some(record.clone()));
        let recovery_run = second_output.join("crawlson-run-recover456");
        fs::create_dir(&recovery_run).unwrap();
        assert_eq!(
            second.resume(record.clone(), &recovery_run).unwrap_err(),
            RecoveryError::Pending,
            "a live main run must hold the origin lock against recovery"
        );
        drop(active);
        let resumed = second
            .resume(
                second.check_pending(&origin).unwrap().unwrap(),
                &recovery_run,
            )
            .unwrap();
        resumed.complete_verified().unwrap();
        assert_eq!(second.check_pending(&origin).unwrap(), None);
        assert!(
            run_root.join(RUN_RECOVERY_FILE).is_file(),
            "the original run marker remains immutable historical evidence"
        );
    }

    #[test]
    fn dropping_active_recovery_never_clears_the_barrier() {
        let (_output, store, run_root, record, origin) = fixture();
        let active = store.begin(record.clone(), &run_root).unwrap();
        drop(active);

        assert_eq!(store.check_pending(&origin).unwrap(), Some(record));
        assert!(run_root.join(RUN_RECOVERY_FILE).is_file());
    }

    #[test]
    fn record_rejects_noncanonical_origin_and_unbounded_or_duplicate_fields() {
        let (_output, _store, _run_root, mut record, _origin) = fixture();
        record.run_id = "AbC123".to_owned();
        record.run_directory = "crawlson-run-AbC123".to_owned();
        assert_eq!(record.validate(), Ok(()));

        record.target_origin = "http://127.0.0.1:4173/path".to_owned();
        assert_eq!(record.validate(), Err(RecoveryError::InvalidRecord));

        record.target_origin = "http://127.0.0.1:4173".to_owned();
        record.cleanup_step_ids = vec!["cleanup".to_owned(), "cleanup".to_owned()];
        assert_eq!(record.validate(), Err(RecoveryError::InvalidRecord));

        record.cleanup_step_ids = (0..=MAX_CLEANUP_STEPS)
            .map(|index| format!("cleanup-{index}"))
            .collect();
        assert_eq!(record.validate(), Err(RecoveryError::InvalidRecord));
    }

    #[test]
    fn tampered_and_oversized_journals_fail_closed() {
        let (_output, store, run_root, record, origin) = fixture();
        let active = store.begin(record, &run_root).unwrap();
        let central = active.central_path.clone();
        drop(active);

        let mut bytes = fs::read(&central).unwrap();
        let position = bytes
            .windows(b"demo.mutate-item".len())
            .position(|window| window == b"demo.mutate-item")
            .unwrap();
        bytes[position] = b'x';
        fs::write(&central, bytes).unwrap();
        assert_eq!(
            store.check_pending(&origin),
            Err(RecoveryError::InvalidJournal)
        );

        fs::write(&central, vec![b'x'; MAX_RECOVERY_BYTES as usize + 1]).unwrap();
        assert_eq!(
            store.check_pending(&origin),
            Err(RecoveryError::InvalidJournal)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_and_hardlinked_journals_fail_closed() {
        use std::os::unix::fs::symlink;

        let (_output, store, run_root, record, origin) = fixture();
        let active = store.begin(record, &run_root).unwrap();
        let central = active.central_path.clone();
        drop(active);

        let saved = central.with_extension("saved");
        fs::rename(&central, &saved).unwrap();
        symlink(&saved, &central).unwrap();
        assert_eq!(
            store.check_pending(&origin),
            Err(RecoveryError::InvalidJournal)
        );

        fs::remove_file(&central).unwrap();
        fs::hard_link(&saved, &central).unwrap();
        assert_eq!(
            store.check_pending(&origin),
            Err(RecoveryError::InvalidJournal)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_store_or_run_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let output = tempfile::tempdir().unwrap();
        let real_store = output.path().join("real-store");
        fs::create_dir(&real_store).unwrap();
        symlink(&real_store, output.path().join(RECOVERY_DIRECTORY)).unwrap();
        assert_eq!(
            RecoveryStore::new(output.path()).unwrap_err(),
            RecoveryError::InvalidStore
        );

        let output = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(output.path()).unwrap();
        let real_run = output.path().join("real-run");
        fs::create_dir(&real_run).unwrap();
        let linked_run = output.path().join("crawlson-run-abc123");
        symlink(&real_run, &linked_run).unwrap();
        let origin = Origin::parse("http://127.0.0.1:4173").unwrap();
        let record = RecoveryRecord {
            schema_version: 1,
            journey_id: "demo.mutate-item".to_owned(),
            revision: 1,
            source_sha256: "a".repeat(64),
            target_origin: origin.to_string(),
            run_id: "abc123".to_owned(),
            run_directory: "crawlson-run-abc123".to_owned(),
            cleanup_step_ids: vec!["cleanup".to_owned()],
            created_at_unix_ms: 1,
        };
        assert_eq!(
            store.begin(record, &linked_run).unwrap_err(),
            RecoveryError::InvalidStore
        );
    }

    #[test]
    fn run_copy_tampering_prevents_completion_and_keeps_origin_blocked() {
        let (_output, store, run_root, record, origin) = fixture();
        let active = store.begin(record.clone(), &run_root).unwrap();
        fs::write(run_root.join(RUN_RECOVERY_FILE), b"{}\n").unwrap();

        assert_eq!(
            active.complete_verified(),
            Err(RecoveryError::CompleteFailed)
        );
        assert_eq!(store.check_pending(&origin).unwrap(), Some(record));
    }

    #[test]
    fn preexisting_run_copy_leaves_no_new_origin_barrier() {
        let (_output, store, run_root, record, origin) = fixture();
        fs::write(run_root.join(RUN_RECOVERY_FILE), b"occupied").unwrap();
        assert_eq!(
            store.begin(record, &run_root).unwrap_err(),
            RecoveryError::Pending
        );
        assert_eq!(store.check_pending(&origin).unwrap(), None);
    }

    #[test]
    fn on_disk_shape_cannot_accept_secret_or_provider_fields() {
        let (_output, _store, _run_root, record, _origin) = fixture();
        let mut value = serde_json::to_value(StoredDocument {
            record,
            record_sha256: "a".repeat(64),
        })
        .unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("fixture_token".to_owned(), serde_json::json!("secret"));
        assert!(serde_json::from_value::<StoredDocument>(value).is_err());
    }
}
