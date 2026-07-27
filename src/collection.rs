use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::journey::{self, ValidatedJourney};
use crate::render::{self, RenderOptions, RenderStatus};
use crate::{CommandResult, VERSION};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_TOPICS: usize = 128;
const MAX_GUIDES: usize = 1024;
const MAX_AUDIENCES: usize = 32;
const MAX_INPUT_FILES: usize = 4096;
const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OUTPUT_FILES: usize = 8192;
const MAX_OUTPUT_BYTES: u64 = 768 * 1024 * 1024;
const MAX_INPUT_DEPTH: usize = 12;
const MAX_ID_BYTES: usize = 96;
const MAX_TITLE_BYTES: usize = 512;
const MAX_DESCRIPTION_BYTES: usize = 16 * 1024;
const MAX_AUDIENCE_BYTES: usize = 256;

pub const EXIT_FINDINGS: u8 = 1;
pub const EXIT_NOT_PUBLISHABLE: u8 = 3;
pub const EXIT_ERROR: u8 = 4;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionMode {
    Build,
    Check,
}

#[derive(Debug, Clone)]
pub struct CollectionOptions {
    pub manifest_path: PathBuf,
    pub output_directory: PathBuf,
    pub mode: CollectionMode,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionStatus {
    Ready,
    Findings,
    NotPublishable,
    Error,
}

impl CollectionStatus {
    fn exit_code(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::Findings => EXIT_FINDINGS,
            Self::NotPublishable => EXIT_NOT_PUBLISHABLE,
            Self::Error => EXIT_ERROR,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Findings => "findings",
            Self::NotPublishable => "not_publishable",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionReport {
    pub schema_version: u8,
    pub crawlson_version: &'static str,
    pub status: CollectionStatus,
    pub publishable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionIdentity>,
    pub summary: CollectionSummary,
    pub entries: Vec<CollectionEntryReport>,
    pub diagnostics: Vec<CollectionDiagnostic>,
    pub outputs: Vec<CollectionOutput>,
}

impl CollectionReport {
    pub fn render(&self, json: bool) -> CommandResult {
        let exit_code = self.status.exit_code();
        if json {
            let mut stdout =
                serde_json::to_string(self).expect("collection report is serializable");
            stdout.push('\n');
            CommandResult {
                exit_code,
                stdout,
                stderr: String::new(),
            }
        } else {
            let mut stdout = format!(
                "Crawlson guide collection: {}\nGuides: {}\nFindings: {}\nUnavailable: {}\n",
                self.status.as_str(),
                self.summary.guides,
                self.summary.findings,
                self.summary.unavailable
            );
            for diagnostic in &self.diagnostics {
                stdout.push_str(&format!("{}: {}\n", diagnostic.code, diagnostic.message));
            }
            CommandResult {
                exit_code,
                stdout,
                stderr: String::new(),
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionIdentity {
    pub id: String,
    pub manifest_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CollectionSummary {
    pub guides: u32,
    pub findings: u32,
    pub unavailable: u32,
    pub errors: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionEntryReport {
    pub key: String,
    pub topic_id: String,
    pub status: RenderStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_sha256: Option<String>,
    pub journey: CollectionJourney,
    pub reason_code: String,
    pub guide_steps: u32,
    pub findings: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionJourney {
    pub id: String,
    pub revision: u32,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct CollectionDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionOutput {
    pub kind: String,
    pub path: String,
    pub size_bytes: u64,
    pub media_type: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journey_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CollectionManifest {
    schema_version: u8,
    collection: CollectionMeta,
    topics: Vec<TopicSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CollectionMeta {
    id: String,
    title: String,
    description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TopicSpec {
    id: String,
    title: String,
    description: String,
    order: u32,
    audience: Vec<String>,
    guides: Vec<GuideSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GuideSpec {
    key: String,
    order: u32,
    run: String,
    journey: String,
}

#[derive(Debug)]
struct PreparedCollection {
    report: CollectionReport,
    files: BTreeMap<String, Vec<u8>>,
    output_directory: PathBuf,
}

#[derive(Debug)]
struct PreparedEntry {
    topic: TopicSpec,
    guide: GuideSpec,
    title: String,
    purpose: String,
    expected_outcome: String,
    verification_scope: String,
    guide_steps: Vec<PreparedGuideStep>,
    render_report: render::RenderReport,
    journey: CollectionJourney,
    files: BTreeMap<String, Vec<u8>>,
    evidence: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
struct PreparedGuideStep {
    id: String,
    title: String,
    instruction: String,
    claim_type: &'static str,
    claim: &'static str,
    alt_text: String,
    source_image: String,
}

#[derive(Debug)]
struct CollectionError {
    code: &'static str,
    message: String,
    entry: Option<String>,
    path: Option<String>,
}

impl CollectionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            entry: None,
            path: None,
        }
    }

    fn entry(mut self, entry: &str) -> Self {
        self.entry = Some(entry.to_owned());
        self
    }

    fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_owned());
        self
    }
}

pub fn run(options: CollectionOptions) -> CollectionReport {
    let prepared = match prepare(&options) {
        Ok(prepared) => prepared,
        Err(error) => return error_report(error),
    };
    if prepared.report.status == CollectionStatus::Error {
        return prepared.report;
    }
    match options.mode {
        CollectionMode::Build => build(prepared),
        CollectionMode::Check => check(prepared),
    }
}

fn error_report(error: CollectionError) -> CollectionReport {
    CollectionReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status: CollectionStatus::Error,
        publishable: false,
        collection: None,
        summary: CollectionSummary {
            errors: 1,
            ..CollectionSummary::default()
        },
        entries: Vec::new(),
        diagnostics: vec![CollectionDiagnostic {
            code: error.code.to_owned(),
            message: error.message,
            entry: error.entry,
            path: error.path,
        }],
        outputs: Vec::new(),
    }
}

fn prepare(options: &CollectionOptions) -> Result<PreparedCollection, CollectionError> {
    let manifest_path =
        resolve_existing_cli_path(&options.manifest_path, false, "manifest_invalid")?;
    let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES, "manifest_invalid")?;
    let manifest_sha256 = journey::hex_digest(&manifest_bytes);
    let mut manifest: CollectionManifest = toml::from_slice(&manifest_bytes).map_err(|_| {
        CollectionError::new(
            "manifest_invalid",
            "collection manifest is not valid strict TOML",
        )
    })?;
    validate_manifest(&manifest)?;
    manifest
        .topics
        .sort_by_key(|topic| (topic.order, topic.id.clone()));
    for topic in &mut manifest.topics {
        topic
            .guides
            .sort_by_key(|guide| (guide.order, guide.key.clone()));
    }

    let workspace = manifest_path
        .parent()
        .expect("a resolved manifest has a parent")
        .to_path_buf();
    let output_directory = resolve_output_path(&options.output_directory)?;

    let mut entries = Vec::new();
    let mut run_ids = HashSet::new();
    let mut journey_ids = HashSet::new();
    let mut input_limits = CopyLimits::default();
    let mut retained_limits = CopyLimits::default();
    for topic in &manifest.topics {
        for guide in &topic.guides {
            let journey_path = resolve_workspace_input(&workspace, &guide.journey, false)?;
            let run_directory = resolve_workspace_input(&workspace, &guide.run, true)?;
            if output_directory.starts_with(&run_directory)
                || run_directory.starts_with(&output_directory)
            {
                return Err(CollectionError::new(
                    "output_overlaps_input",
                    "collection output and a run input must not overlap",
                )
                .entry(&guide.key));
            }
            let loaded = journey::load(&journey_path).map_err(|_| {
                CollectionError::new(
                    "journey_invalid",
                    "entry journey could not be loaded as a bounded journey document",
                )
                .entry(&guide.key)
            })?;
            let validated = journey::validate(loaded).map_err(|_| {
                CollectionError::new(
                    "journey_invalid",
                    "entry journey did not satisfy the Crawlson journey contract",
                )
                .entry(&guide.key)
            })?;
            validate_collection_journey_text(&validated, &guide.key)?;
            if !journey_ids.insert(validated.meta.id.clone()) {
                return Err(CollectionError::new(
                    "journey_identity_duplicate",
                    "one collection may contain only one active entry for a journey identity",
                )
                .entry(&guide.key));
            }
            let prepared = prepare_entry(
                topic.clone(),
                guide.clone(),
                &run_directory,
                &journey_path,
                &validated,
                &mut input_limits,
                &mut retained_limits,
            )?;
            if let Some(run_id) = &prepared.render_report.run_id
                && !run_ids.insert(run_id.clone())
            {
                return Err(CollectionError::new(
                    "run_identity_duplicate",
                    "one run identity cannot satisfy more than one collection entry",
                )
                .entry(&guide.key));
            }
            entries.push(prepared);
        }
    }

    let mut summary = CollectionSummary::default();
    let mut entry_reports = Vec::new();
    let mut has_not_publishable = false;
    let mut has_findings = false;
    let mut has_error = false;
    for entry in &entries {
        match entry.render_report.status {
            RenderStatus::GuideReady => summary.guides += 1,
            RenderStatus::FindingsReady => {
                summary.findings += entry.render_report.findings;
                has_findings = true;
            }
            RenderStatus::NotPublishable => {
                summary.unavailable += 1;
                has_not_publishable = true;
            }
            RenderStatus::Error => {
                summary.errors += 1;
                has_error = true;
            }
        }
        entry_reports.push(CollectionEntryReport {
            key: entry.guide.key.clone(),
            topic_id: entry.topic.id.clone(),
            status: entry.render_report.status,
            run_id: entry.render_report.run_id.clone(),
            report_sha256: entry.render_report.report_sha256.clone(),
            journey: entry.journey.clone(),
            reason_code: entry.render_report.reason.code.clone(),
            guide_steps: entry.render_report.guide_steps,
            findings: entry.render_report.findings,
        });
    }
    let status = if has_error {
        CollectionStatus::Error
    } else if has_not_publishable {
        CollectionStatus::NotPublishable
    } else if has_findings {
        CollectionStatus::Findings
    } else {
        CollectionStatus::Ready
    };

    let mut output_limits = retained_limits;
    let mut files = if status == CollectionStatus::Error {
        BTreeMap::new()
    } else if status == CollectionStatus::Ready {
        build_public_files(&manifest, &entries, &manifest_sha256, &mut output_limits)?
    } else {
        build_review_files(&manifest, &entries, status, &mut output_limits)?
    };
    let outputs = output_records(&files, &entries);
    let snapshot_sha256 = snapshot_digest(&manifest_sha256, &entry_reports, &outputs);
    let report = CollectionReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status,
        publishable: status == CollectionStatus::Ready,
        collection: Some(CollectionIdentity {
            id: manifest.collection.id.clone(),
            manifest_sha256,
            snapshot_sha256: Some(snapshot_sha256),
        }),
        summary,
        entries: entry_reports,
        diagnostics: diagnostics_for_entries(status, &entries),
        outputs,
    };
    if status == CollectionStatus::Error {
        return Ok(PreparedCollection {
            report,
            files,
            output_directory,
        });
    }
    let mut report_bytes = serde_json::to_vec_pretty(&report).map_err(|_| {
        CollectionError::new(
            "collection_serialize_failed",
            "collection report could not be serialized",
        )
    })?;
    report_bytes.push(b'\n');
    insert_owned_bounded(
        &mut files,
        &mut output_limits,
        "collection-report.json".to_owned(),
        report_bytes,
        "generated collection",
    )?;
    audit_generated_tree(&files)?;

    Ok(PreparedCollection {
        report,
        files,
        output_directory,
    })
}

fn validate_manifest(manifest: &CollectionManifest) -> Result<(), CollectionError> {
    if manifest.schema_version != 1 {
        return Err(CollectionError::new(
            "manifest_version_unsupported",
            "only collection manifest schema version 1 is supported",
        ));
    }
    validate_id(&manifest.collection.id, "collection id")?;
    validate_text(
        &manifest.collection.title,
        MAX_TITLE_BYTES,
        "collection title",
    )?;
    validate_text(
        &manifest.collection.description,
        MAX_DESCRIPTION_BYTES,
        "collection description",
    )?;
    if manifest.topics.is_empty() || manifest.topics.len() > MAX_TOPICS {
        return Err(CollectionError::new(
            "manifest_invalid",
            format!("collection must contain 1 to {MAX_TOPICS} topics"),
        ));
    }
    let mut topic_ids = HashSet::new();
    let mut topic_orders = HashSet::new();
    let mut guide_keys = HashSet::new();
    let mut total_guides = 0usize;
    for topic in &manifest.topics {
        validate_id(&topic.id, "topic id")?;
        validate_text(&topic.title, MAX_TITLE_BYTES, "topic title")?;
        validate_text(
            &topic.description,
            MAX_DESCRIPTION_BYTES,
            "topic description",
        )?;
        if !topic_ids.insert(topic.id.clone()) {
            return Err(CollectionError::new(
                "topic_duplicate",
                "topic identifiers must be unique",
            ));
        }
        if !topic_orders.insert(topic.order) {
            return Err(CollectionError::new(
                "topic_order_duplicate",
                "topic order values must be unique",
            ));
        }
        if topic.audience.len() > MAX_AUDIENCES {
            return Err(CollectionError::new(
                "manifest_invalid",
                format!("a topic may contain at most {MAX_AUDIENCES} audience labels"),
            ));
        }
        let mut audiences = HashSet::new();
        for audience in &topic.audience {
            validate_text(audience, MAX_AUDIENCE_BYTES, "audience label")?;
            if !audiences.insert(audience.clone()) {
                return Err(CollectionError::new(
                    "audience_duplicate",
                    "audience labels must be unique within a topic",
                ));
            }
        }
        if topic.guides.is_empty() {
            return Err(CollectionError::new(
                "manifest_invalid",
                "every topic must contain at least one guide entry",
            ));
        }
        let mut guide_orders = HashSet::new();
        for guide in &topic.guides {
            validate_id(&guide.key, "guide key")?;
            if guide.key == "index.md" {
                return Err(CollectionError::new(
                    "manifest_invalid",
                    "guide key 'index.md' would collide with its topic index",
                )
                .entry(&guide.key));
            }
            if !guide_keys.insert(guide.key.clone()) {
                return Err(CollectionError::new(
                    "guide_key_duplicate",
                    "guide keys must be unique across the collection",
                ));
            }
            if !guide_orders.insert(guide.order) {
                return Err(CollectionError::new(
                    "guide_order_duplicate",
                    "guide order values must be unique within a topic",
                )
                .entry(&guide.key));
            }
            validate_portable_relative(&guide.run, "run path")?;
            validate_portable_relative(&guide.journey, "journey path")?;
            total_guides += 1;
        }
    }
    if total_guides > MAX_GUIDES {
        return Err(CollectionError::new(
            "manifest_invalid",
            format!("collection may contain at most {MAX_GUIDES} guide entries"),
        ));
    }
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<(), CollectionError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !portable_segment(value)
    {
        return Err(CollectionError::new(
            "manifest_invalid",
            format!("{label} must be a bounded lowercase portable identifier"),
        ));
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize, label: &str) -> Result<(), CollectionError> {
    if value.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(CollectionError::new(
            "manifest_invalid",
            format!("{label} must contain 1 to {maximum} characters without controls"),
        ));
    }
    Ok(())
}

fn validate_collection_journey_text(
    journey: &ValidatedJourney,
    entry: &str,
) -> Result<(), CollectionError> {
    let values = [
        (
            journey.meta.title.as_str(),
            MAX_DESCRIPTION_BYTES,
            "journey title",
        ),
        (
            journey.meta.purpose.as_str(),
            MAX_DESCRIPTION_BYTES,
            "journey purpose",
        ),
        (
            journey.meta.expected_outcome.as_str(),
            MAX_DESCRIPTION_BYTES,
            "journey expected outcome",
        ),
    ];
    for (value, maximum, label) in values {
        if value.is_empty()
            || value.chars().count() > maximum
            || value.chars().any(char::is_control)
        {
            return Err(CollectionError::new(
                "journey_invalid",
                format!("{label} is not portable into the collection document"),
            )
            .entry(entry));
        }
    }
    for step in &journey.steps {
        if step.title.is_empty()
            || step.title.chars().count() > MAX_DESCRIPTION_BYTES
            || step.title.chars().any(char::is_control)
        {
            return Err(CollectionError::new(
                "journey_invalid",
                "journey step title is not portable into the collection document",
            )
            .entry(entry));
        }
    }
    Ok(())
}

fn validate_portable_relative(value: &str, label: &str) -> Result<(), CollectionError> {
    if value.is_empty()
        || value.chars().count() > 4096
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || !portable_segment(part))
    {
        return Err(CollectionError::new(
            "manifest_path_invalid",
            format!("{label} must be a bounded portable relative path"),
        ));
    }
    Ok(())
}

fn portable_segment(value: &str) -> bool {
    if value.is_empty()
        || value.ends_with('.')
        || value.ends_with(' ')
        || value.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return false;
    }
    let stem = value
        .split('.')
        .next()
        .expect("a non-empty segment has a stem")
        .to_ascii_lowercase();
    !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        && !(stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn resolve_existing_cli_path(
    path: &Path,
    directory: bool,
    code: &'static str,
) -> Result<PathBuf, CollectionError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| CollectionError::new(code, "current directory is unavailable"))?
            .join(path)
    };
    let path_metadata = fs::symlink_metadata(&absolute)
        .map_err(|_| CollectionError::new(code, "required input does not exist"))?;
    if path_metadata.file_type().is_symlink() {
        return Err(CollectionError::new(
            code,
            "the requested input itself may not be a symlink",
        ));
    }
    let metadata = fs::metadata(&absolute)
        .map_err(|_| CollectionError::new(code, "required input does not exist"))?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(CollectionError::new(
            code,
            "input has the wrong filesystem type",
        ));
    }
    absolute
        .canonicalize()
        .map_err(|_| CollectionError::new(code, "input could not be resolved"))
}

fn reject_symlink_components(path: &Path, code: &'static str) -> Result<(), CollectionError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::RootDir | Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CollectionError::new(
                    code,
                    "symlinked path components are not allowed",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => {
                return Err(CollectionError::new(
                    code,
                    "path metadata could not be inspected",
                ));
            }
        }
    }
    Ok(())
}

fn resolve_workspace_input(
    workspace: &Path,
    relative: &str,
    directory: bool,
) -> Result<PathBuf, CollectionError> {
    validate_portable_relative(relative, "input path")?;
    let candidate = workspace.join(relative);
    reject_symlink_components(&candidate, "manifest_path_invalid")?;
    let metadata = fs::metadata(&candidate).map_err(|_| {
        CollectionError::new(
            "manifest_input_missing",
            "a declared collection input is missing",
        )
    })?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(CollectionError::new(
            "manifest_path_invalid",
            "a declared collection input has the wrong filesystem type",
        ));
    }
    let resolved = candidate.canonicalize().map_err(|_| {
        CollectionError::new(
            "manifest_path_invalid",
            "a declared collection input could not be resolved",
        )
    })?;
    if !resolved.starts_with(workspace) {
        return Err(CollectionError::new(
            "manifest_path_escape",
            "declared collection inputs must remain inside the manifest workspace",
        ));
    }
    Ok(resolved)
}

fn resolve_output_path(path: &Path) -> Result<PathBuf, CollectionError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| {
                CollectionError::new("output_invalid", "current directory is unavailable")
            })?
            .join(path)
    };
    if absolute.file_name().is_none() {
        return Err(CollectionError::new(
            "output_invalid",
            "collection output must name a directory beneath an existing parent",
        ));
    }
    let parent = absolute.parent().ok_or_else(|| {
        CollectionError::new(
            "output_invalid",
            "collection output must have an existing parent",
        )
    })?;
    let parent = resolve_existing_cli_path(parent, true, "output_invalid")?;
    let output = parent.join(
        absolute
            .file_name()
            .expect("an output filename was checked above"),
    );
    if output.exists() {
        reject_symlink_components(&output, "output_invalid")?;
        if !fs::metadata(&output)
            .map_err(|_| CollectionError::new("output_invalid", "output is unavailable"))?
            .is_dir()
        {
            return Err(CollectionError::new(
                "output_invalid",
                "existing collection output must be a real directory",
            ));
        }
    }
    Ok(output)
}

fn read_bounded(path: &Path, maximum: u64, code: &'static str) -> Result<Vec<u8>, CollectionError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CollectionError::new(code, "file metadata is unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(CollectionError::new(
            code,
            "file is not a bounded regular file",
        ));
    }
    let mut file =
        fs::File::open(path).map_err(|_| CollectionError::new(code, "file could not be opened"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CollectionError::new(code, "file could not be read"))?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > maximum {
        return Err(CollectionError::new(
            code,
            "file changed or exceeded its bound while being read",
        ));
    }
    Ok(bytes)
}

fn read_bounded_accounted(
    path: &Path,
    maximum: u64,
    code: &'static str,
    limits: &mut CopyLimits,
    maximum_files: usize,
    maximum_bytes: u64,
) -> Result<Vec<u8>, CollectionError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CollectionError::new(code, "file metadata is unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(CollectionError::new(
            code,
            "file is not a bounded regular file",
        ));
    }
    limits.reserve(metadata.len(), maximum_files, maximum_bytes, code)?;
    read_bounded(path, maximum, code)
}

fn prepare_entry(
    topic: TopicSpec,
    guide: GuideSpec,
    run_directory: &Path,
    journey_path: &Path,
    journey: &ValidatedJourney,
    input_limits: &mut CopyLimits,
    retained_limits: &mut CopyLimits,
) -> Result<PreparedEntry, CollectionError> {
    let mirror = tempfile::Builder::new()
        .prefix(".crawlson-collection-input-")
        .tempdir()
        .map_err(|_| {
            CollectionError::new(
                "input_snapshot_failed",
                "temporary input snapshot could not be created",
            )
            .entry(&guide.key)
        })?;
    let mirror_root = mirror.path().join("run");
    fs::create_dir(&mirror_root).map_err(|_| {
        CollectionError::new(
            "input_snapshot_failed",
            "temporary run directory could not be created",
        )
        .entry(&guide.key)
    })?;
    copy_run_tree(run_directory, &mirror_root, 0, true, input_limits)
        .map_err(|error| error.entry(&guide.key))?;
    let render_report = render::run(RenderOptions {
        run_directory: mirror_root.clone(),
        journey_path: journey_path.to_path_buf(),
    });
    if let Some(rendered_journey) = render_report.journey.as_ref() {
        if rendered_journey.id != journey.meta.id
            || rendered_journey.revision != journey.meta.revision
            || rendered_journey.source_sha256 != journey.source_sha256
        {
            return Err(CollectionError::new(
                "entry_provenance_mismatch",
                "entry renderer returned conflicting journey provenance",
            )
            .entry(&guide.key));
        }
    } else if render_report.status != RenderStatus::Error {
        return Err(CollectionError::new(
            "entry_render_error",
            "entry renderer did not retain journey provenance",
        )
        .entry(&guide.key));
    }
    let mut files = BTreeMap::new();
    for output in &render_report.outputs {
        validate_portable_relative(&output.path, "render output path")?;
        let bytes = read_bounded_accounted(
            &mirror_root.join(&output.path),
            MAX_INPUT_BYTES,
            "entry_render_output_invalid",
            retained_limits,
            MAX_OUTPUT_FILES,
            MAX_OUTPUT_BYTES,
        )?;
        if bytes.len() as u64 != output.size_bytes || journey::hex_digest(&bytes) != output.sha256 {
            return Err(CollectionError::new(
                "entry_render_output_invalid",
                "entry render output failed its recorded digest or size",
            )
            .entry(&guide.key));
        }
        files.insert(output.path.clone(), bytes);
    }
    let mut evidence = BTreeMap::new();
    if render_report.status == RenderStatus::FindingsReady {
        let findings = files.get("render/findings.json").ok_or_else(|| {
            CollectionError::new(
                "entry_render_output_invalid",
                "findings-ready entry omitted findings JSON",
            )
            .entry(&guide.key)
        })?;
        for relative in finding_evidence_paths(findings)? {
            let bytes = read_bounded_accounted(
                &mirror_root.join(&relative),
                MAX_INPUT_BYTES,
                "entry_evidence_invalid",
                retained_limits,
                MAX_OUTPUT_FILES,
                MAX_OUTPUT_BYTES,
            )?;
            evidence.insert(relative, bytes);
        }
    }
    let image_outputs = render_report
        .outputs
        .iter()
        .filter(|output| output.kind == "guide_image")
        .collect::<Vec<_>>();
    let declared_steps = journey
        .steps
        .iter()
        .filter(|step| step.guide_instruction.is_some())
        .collect::<Vec<_>>();
    let guide_steps = if render_report.status == RenderStatus::GuideReady {
        if image_outputs.len() != declared_steps.len()
            || image_outputs.len() != render_report.guide_steps as usize
        {
            return Err(CollectionError::new(
                "entry_render_output_invalid",
                "guide step and focused image counts do not match",
            )
            .entry(&guide.key));
        }
        declared_steps
            .into_iter()
            .zip(image_outputs)
            .map(|(step, image)| {
                let (alt_text, claim_type, claim) = match &step.action {
                    crate::journey::ValidatedAction::Capture { alt_text, .. } => (
                        alt_text.clone(),
                        "observed_next_action",
                        "The highlighted action area was observed in the read-only run. The authored instruction describes the reader's next action; Crawlson does not claim that action was executed.",
                    ),
                    crate::journey::ValidatedAction::FollowLink { alt_text, .. } => (
                        alt_text.clone(),
                        "executed_and_verified",
                        "Crawlson executed this highlighted link action once and verified its exact declared same-origin destination.",
                    ),
                    _ => unreachable!("guide-ready steps are focused evidence actions"),
                };
                PreparedGuideStep {
                    id: step.id.clone(),
                    title: step.title.clone(),
                    instruction: step
                        .guide_instruction
                        .clone()
                        .expect("filtered guide instruction is present"),
                    claim_type,
                    claim,
                    alt_text,
                    source_image: image.path.clone(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(PreparedEntry {
        topic,
        guide,
        title: journey.meta.title.clone(),
        purpose: journey.meta.purpose.clone(),
        expected_outcome: journey.meta.expected_outcome.clone(),
        verification_scope: if journey.schema_version >= 3 {
            "the declared checkpoints, authorized actions, and focused evidence".to_owned()
        } else {
            "the declared checkpoints and captures".to_owned()
        },
        guide_steps,
        render_report,
        journey: CollectionJourney {
            id: journey.meta.id.clone(),
            revision: journey.meta.revision,
            source_sha256: journey.source_sha256.clone(),
        },
        files,
        evidence,
    })
}

#[derive(Debug, Default)]
struct CopyLimits {
    files: usize,
    bytes: u64,
}

impl CopyLimits {
    fn reserve(
        &mut self,
        bytes: u64,
        maximum_files: usize,
        maximum_bytes: u64,
        code: &'static str,
    ) -> Result<(), CollectionError> {
        let files = self
            .files
            .checked_add(1)
            .ok_or_else(|| CollectionError::new(code, "file count overflowed"))?;
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| CollectionError::new(code, "byte count overflowed"))?;
        if files > maximum_files || total > maximum_bytes {
            return Err(CollectionError::new(
                code,
                "collection exceeds its aggregate file or byte bounds",
            ));
        }
        self.files = files;
        self.bytes = total;
        Ok(())
    }
}

fn copy_run_tree(
    source: &Path,
    destination: &Path,
    depth: usize,
    top_level: bool,
    limits: &mut CopyLimits,
) -> Result<(), CollectionError> {
    if depth > MAX_INPUT_DEPTH {
        return Err(CollectionError::new(
            "input_snapshot_failed",
            "run directory nesting exceeds the collection bound",
        ));
    }
    let mut entries = fs::read_dir(source)
        .map_err(|_| {
            CollectionError::new(
                "input_snapshot_failed",
                "run directory could not be enumerated",
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            CollectionError::new(
                "input_snapshot_failed",
                "run directory entry could not be inspected",
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name_text = name.to_str().ok_or_else(|| {
            CollectionError::new(
                "input_snapshot_failed",
                "run directory contains a non-portable filename",
            )
        })?;
        if top_level && name_text == "render" {
            continue;
        }
        if !portable_segment(name_text) {
            return Err(CollectionError::new(
                "input_snapshot_failed",
                "run directory contains a non-portable filename",
            ));
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| {
            CollectionError::new(
                "input_snapshot_failed",
                "run entry metadata could not be inspected",
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(CollectionError::new(
                "input_symlink_rejected",
                "run inputs may not contain symlinks",
            ));
        }
        let target = destination.join(&name);
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|_| {
                CollectionError::new(
                    "input_snapshot_failed",
                    "temporary run directory could not be created",
                )
            })?;
            copy_run_tree(&entry.path(), &target, depth + 1, false, limits)?;
        } else if metadata.is_file() {
            limits.reserve(
                metadata.len(),
                MAX_INPUT_FILES,
                MAX_INPUT_BYTES,
                "input_snapshot_failed",
            )?;
            let copied = fs::copy(entry.path(), &target).map_err(|_| {
                CollectionError::new(
                    "input_snapshot_failed",
                    "run input could not be copied into the temporary snapshot",
                )
            })?;
            if copied != metadata.len() {
                return Err(CollectionError::new(
                    "input_snapshot_failed",
                    "run input changed while the snapshot was copied",
                ));
            }
        } else {
            return Err(CollectionError::new(
                "input_snapshot_failed",
                "run inputs must contain only regular files and directories",
            ));
        }
    }
    Ok(())
}

fn finding_evidence_paths(bytes: &[u8]) -> Result<BTreeSet<String>, CollectionError> {
    let document: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| {
        CollectionError::new(
            "entry_render_output_invalid",
            "findings output is not valid JSON",
        )
    })?;
    let findings = document
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            CollectionError::new(
                "entry_render_output_invalid",
                "findings output omitted its findings array",
            )
        })?;
    let mut paths = BTreeSet::new();
    for finding in findings {
        let evidence = finding
            .get("evidence")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                CollectionError::new(
                    "entry_render_output_invalid",
                    "finding omitted its evidence array",
                )
            })?;
        for item in evidence {
            let path = item
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    CollectionError::new(
                        "entry_render_output_invalid",
                        "finding evidence omitted its path",
                    )
                })?;
            validate_portable_relative(path, "finding evidence path")?;
            paths.insert(path.to_owned());
        }
    }
    Ok(paths)
}

fn build_public_files(
    manifest: &CollectionManifest,
    entries: &[PreparedEntry],
    manifest_sha256: &str,
    limits: &mut CopyLimits,
) -> Result<BTreeMap<String, Vec<u8>>, CollectionError> {
    let mut files = BTreeMap::new();
    insert_owned_bounded(
        &mut files,
        limits,
        "index.md".to_owned(),
        root_index_markdown(manifest).into_bytes(),
        "generated collection",
    )?;
    let mut app_topics = Vec::new();
    for topic in &manifest.topics {
        let topic_entries = entries
            .iter()
            .filter(|entry| entry.topic.id == topic.id)
            .collect::<Vec<_>>();
        insert_owned_bounded(
            &mut files,
            limits,
            format!("topics/{}/index.md", topic.id),
            topic_index_markdown(topic, &topic_entries).into_bytes(),
            "generated collection",
        )?;
        let mut app_guides = Vec::new();
        for (index, entry) in topic_entries.iter().enumerate() {
            if entry.render_report.status != RenderStatus::GuideReady {
                return Err(CollectionError::new(
                    "public_collection_incomplete",
                    "public guide output requires every entry to be guide-ready",
                )
                .entry(&entry.guide.key));
            }
            let previous = index
                .checked_sub(1)
                .and_then(|position| topic_entries.get(position).map(|item| &item.guide.key));
            let next = topic_entries.get(index + 1).map(|item| &item.guide.key);
            let page = format!("topics/{}/{}/index.md", topic.id, entry.guide.key);
            let mut images = Vec::new();
            let mut steps = Vec::new();
            for (step_index, step) in entry.guide_steps.iter().enumerate() {
                let source = entry.files.get(&step.source_image).ok_or_else(|| {
                    CollectionError::new(
                        "entry_render_output_invalid",
                        "guide image bytes are missing",
                    )
                    .entry(&entry.guide.key)
                })?;
                let name = Path::new(&step.source_image)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| {
                        CollectionError::new(
                            "entry_render_output_invalid",
                            "guide image path is not portable",
                        )
                        .entry(&entry.guide.key)
                    })?;
                let destination = format!("topics/{}/{}/{}", topic.id, entry.guide.key, name);
                insert_cloned_bounded(
                    &mut files,
                    limits,
                    destination.clone(),
                    source,
                    "generated collection",
                )?;
                let image = AppImage {
                    path: destination,
                    size_bytes: source.len() as u64,
                    sha256: journey::hex_digest(source),
                };
                images.push(image.clone());
                steps.push(AppGuideStep {
                    id: step.id.clone(),
                    number: step_index as u32 + 1,
                    title: step.title.clone(),
                    instruction: step.instruction.clone(),
                    claim_type: step.claim_type,
                    claim: step.claim,
                    alt_text: step.alt_text.clone(),
                    image,
                });
            }
            let mut app_guide = AppGuide {
                key: entry.guide.key.clone(),
                title: entry.title.clone(),
                purpose: entry.purpose.clone(),
                expected_outcome: entry.expected_outcome.clone(),
                verification_scope: entry.verification_scope.clone(),
                order: entry.guide.order,
                topic_id: topic.id.clone(),
                topic_title: topic.title.clone(),
                audience: topic.audience.clone(),
                page,
                page_size_bytes: 0,
                page_sha256: String::new(),
                run_id: entry
                    .render_report
                    .run_id
                    .clone()
                    .expect("guide-ready renders retain a run id"),
                report_sha256: entry
                    .render_report
                    .report_sha256
                    .clone()
                    .expect("guide-ready renders retain a report digest"),
                journey: entry.journey.clone(),
                steps,
                images,
            };
            let page_bytes = guide_page_markdown(topic, &app_guide, previous, next).into_bytes();
            app_guide.page_size_bytes = page_bytes.len() as u64;
            app_guide.page_sha256 = journey::hex_digest(&page_bytes);
            insert_owned_bounded(
                &mut files,
                limits,
                app_guide.page.clone(),
                page_bytes,
                "generated collection",
            )?;
            app_guides.push(app_guide);
        }
        app_topics.push(AppTopic {
            id: topic.id.clone(),
            title: topic.title.clone(),
            description: topic.description.clone(),
            order: topic.order,
            audience: topic.audience.clone(),
            index: format!("topics/{}/index.md", topic.id),
            guides: app_guides,
        });
    }
    let content_outputs = output_records(&files, entries);
    let snapshot_sha256 =
        snapshot_digest(manifest_sha256, &entry_reports(entries), &content_outputs);
    let app = AppCollection {
        schema_version: 1,
        crawlson_version: VERSION,
        collection: AppCollectionMeta {
            id: manifest.collection.id.clone(),
            title: manifest.collection.title.clone(),
            description: manifest.collection.description.clone(),
            manifest_sha256: manifest_sha256.to_owned(),
            snapshot_sha256,
        },
        topics: app_topics,
    };
    let mut bytes = serde_json::to_vec_pretty(&app).map_err(|_| {
        CollectionError::new(
            "collection_serialize_failed",
            "application collection document could not be serialized",
        )
    })?;
    bytes.push(b'\n');
    insert_owned_bounded(
        &mut files,
        limits,
        "guide-collection.json".to_owned(),
        bytes,
        "generated collection",
    )?;
    Ok(files)
}

#[derive(Debug, Serialize)]
struct AppCollection {
    schema_version: u8,
    crawlson_version: &'static str,
    collection: AppCollectionMeta,
    topics: Vec<AppTopic>,
}

#[derive(Debug, Serialize)]
struct AppCollectionMeta {
    id: String,
    title: String,
    description: String,
    manifest_sha256: String,
    snapshot_sha256: String,
}

#[derive(Debug, Serialize)]
struct AppTopic {
    id: String,
    title: String,
    description: String,
    order: u32,
    audience: Vec<String>,
    index: String,
    guides: Vec<AppGuide>,
}

#[derive(Debug, Serialize)]
struct AppGuide {
    key: String,
    title: String,
    purpose: String,
    expected_outcome: String,
    verification_scope: String,
    order: u32,
    topic_id: String,
    topic_title: String,
    audience: Vec<String>,
    page: String,
    page_size_bytes: u64,
    page_sha256: String,
    run_id: String,
    report_sha256: String,
    journey: CollectionJourney,
    steps: Vec<AppGuideStep>,
    images: Vec<AppImage>,
}

#[derive(Debug, Clone, Serialize)]
struct AppImage {
    path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct AppGuideStep {
    id: String,
    number: u32,
    title: String,
    instruction: String,
    claim_type: &'static str,
    claim: &'static str,
    alt_text: String,
    image: AppImage,
}

fn root_index_markdown(manifest: &CollectionManifest) -> String {
    let mut markdown = format!(
        "# {}\n\n{}\n\nThese guides were generated only from complete, verified Crawlson journeys.\n\n## Topics\n",
        escape_markdown(&manifest.collection.title),
        escape_markdown(&manifest.collection.description)
    );
    for topic in &manifest.topics {
        let audience = if topic.audience.is_empty() {
            String::new()
        } else {
            format!(
                " Audience: {}.",
                topic
                    .audience
                    .iter()
                    .map(|value| escape_markdown(value))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        markdown.push_str(&format!(
            "\n- [{}](topics/{}/index.md) — {}{}\n",
            escape_markdown(&topic.title),
            topic.id,
            escape_markdown(&topic.description),
            audience
        ));
    }
    markdown
}

fn topic_index_markdown(topic: &TopicSpec, entries: &[&PreparedEntry]) -> String {
    let mut markdown = format!(
        "[← All topics](../../index.md)\n\n# {}\n\n{}\n",
        escape_markdown(&topic.title),
        escape_markdown(&topic.description)
    );
    if !topic.audience.is_empty() {
        markdown.push_str(&format!(
            "\nAudience: {}\n",
            topic
                .audience
                .iter()
                .map(|value| escape_markdown(value))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    markdown.push_str("\n## Guides\n");
    for (index, entry) in entries.iter().enumerate() {
        markdown.push_str(&format!(
            "\n{}. [{}]({}/index.md)\n",
            index + 1,
            escape_markdown(&entry.title),
            entry.guide.key
        ));
    }
    markdown
}

fn guide_page_markdown(
    topic: &TopicSpec,
    guide: &AppGuide,
    previous: Option<&String>,
    next: Option<&String>,
) -> String {
    let mut markdown = format!(
        "[← {}](../index.md)\n\n# {}\n\n{}\n\nDeclared expected outcome: {}\n",
        escape_markdown(&topic.title),
        escape_markdown(&guide.title),
        escape_markdown(&guide.purpose),
        escape_markdown(&guide.expected_outcome),
    );
    if !guide.audience.is_empty() {
        markdown.push_str(&format!(
            "\nAudience: {}\n",
            guide
                .audience
                .iter()
                .map(|value| escape_markdown(value))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    markdown.push_str(&format!(
        "\nCrawlson run `{}` passed {} for journey `{}` revision {} (`{}`). Free-form outcome prose is authored context, not an additional executed assertion.\n",
        escape_code(&guide.run_id),
        escape_markdown(&guide.verification_scope),
        escape_code(&guide.journey.id),
        guide.journey.revision,
        guide.journey.source_sha256,
    ));
    for step in &guide.steps {
        let image = Path::new(&step.image.path)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("generated image paths have portable filenames");
        markdown.push_str(&format!(
            "\n## {}. {}\n\n{}\n\n![{}]({})\n\n{}\n\nEvidence SHA-256: `{}`\n",
            step.number,
            escape_markdown(&step.title),
            escape_markdown(&step.instruction),
            escape_markdown(&step.alt_text),
            image,
            step.claim,
            step.image.sha256,
        ));
    }
    markdown.push_str("\n---\n\n");
    if let Some(previous) = previous {
        markdown.push_str(&format!("[← Previous guide](../{previous}/index.md) · "));
    }
    markdown.push_str("[Topic index](../index.md)");
    if let Some(next) = next {
        markdown.push_str(&format!(" · [Next guide →](../{next}/index.md)"));
    }
    markdown.push('\n');
    markdown
}

fn build_review_files(
    manifest: &CollectionManifest,
    entries: &[PreparedEntry],
    status: CollectionStatus,
    limits: &mut CopyLimits,
) -> Result<BTreeMap<String, Vec<u8>>, CollectionError> {
    let mut files = BTreeMap::new();
    let mut markdown = format!(
        "# Review: {}\n\n{}\n\nThis collection is **not publishable**. No reader-facing guide index was emitted.\n",
        escape_markdown(&manifest.collection.title),
        match status {
            CollectionStatus::Findings => {
                "One or more current journeys produced deterministic evidence-backed findings."
            }
            CollectionStatus::NotPublishable => {
                "One or more current journeys were blocked or otherwise unavailable."
            }
            _ => "The collection requires review.",
        }
    );
    markdown.push_str("\n## Entry status\n");
    for entry in entries {
        match entry.render_report.status {
            RenderStatus::GuideReady => markdown.push_str(&format!(
                "\n- **{}** — verified, but withheld because the complete collection is not publishable.\n",
                escape_markdown(&entry.title)
            )),
            RenderStatus::FindingsReady => {
                let prefix = format!("review/{}/{}/", entry.topic.id, entry.guide.key);
                for (path, bytes) in &entry.files {
                    if matches!(path.as_str(), "render/findings.json" | "render/findings.md") {
                        insert_cloned_bounded(
                            &mut files,
                            limits,
                            format!("{prefix}{path}"),
                            bytes,
                            "generated collection",
                        )?;
                    }
                }
                for (path, bytes) in &entry.evidence {
                    insert_cloned_bounded(
                        &mut files,
                        limits,
                        format!("{prefix}{path}"),
                        bytes,
                        "generated collection",
                    )?;
                }
                markdown.push_str(&format!(
                    "\n- **{}** — {} finding(s): [review findings]({}/{}/render/findings.md).\n",
                    escape_markdown(&entry.title),
                    entry.render_report.findings,
                    entry.topic.id,
                    entry.guide.key
                ));
            }
            RenderStatus::NotPublishable => markdown.push_str(&format!(
                "\n- **{}** — unavailable (`{}`).\n",
                escape_markdown(&entry.title),
                entry.render_report.reason.code
            )),
            RenderStatus::Error => unreachable!("error collections emit no filesystem output"),
        }
    }
    insert_owned_bounded(
        &mut files,
        limits,
        "review/index.md".to_owned(),
        markdown.into_bytes(),
        "generated collection",
    )?;
    Ok(files)
}

fn insert_cloned_bounded(
    files: &mut BTreeMap<String, Vec<u8>>,
    limits: &mut CopyLimits,
    path: String,
    bytes: &[u8],
    _label: &'static str,
) -> Result<(), CollectionError> {
    reserve_output_path(files, &path)?;
    limits.reserve(
        bytes.len() as u64,
        MAX_OUTPUT_FILES,
        MAX_OUTPUT_BYTES,
        "output_bounds_exceeded",
    )?;
    files.insert(path, bytes.to_vec());
    Ok(())
}

fn insert_owned_bounded(
    files: &mut BTreeMap<String, Vec<u8>>,
    limits: &mut CopyLimits,
    path: String,
    bytes: Vec<u8>,
    _label: &'static str,
) -> Result<(), CollectionError> {
    reserve_output_path(files, &path)?;
    limits.reserve(
        bytes.len() as u64,
        MAX_OUTPUT_FILES,
        MAX_OUTPUT_BYTES,
        "output_bounds_exceeded",
    )?;
    files.insert(path, bytes);
    Ok(())
}

fn reserve_output_path(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<(), CollectionError> {
    validate_portable_relative(path, "generated output path")?;
    let prefix = format!("{path}/");
    if files.contains_key(path)
        || files.keys().any(|existing| {
            existing.starts_with(&prefix) || path.starts_with(&format!("{existing}/"))
        })
    {
        return Err(CollectionError::new(
            "generated_path_collision",
            "generated output paths collide as files and directories",
        )
        .path(path));
    }
    Ok(())
}

fn escape_markdown(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if matches!(
                character,
                '\\' | '`'
                    | '*'
                    | '_'
                    | '{'
                    | '}'
                    | '['
                    | ']'
                    | '<'
                    | '>'
                    | '#'
                    | '+'
                    | '-'
                    | '.'
                    | '!'
                    | '|'
                    | '('
                    | ')'
            ) {
                vec!['\\', character]
            } else {
                vec![character]
            }
        })
        .collect()
}

fn escape_code(value: &str) -> String {
    value.replace('`', "'")
}

fn output_records(
    files: &BTreeMap<String, Vec<u8>>,
    entries: &[PreparedEntry],
) -> Vec<CollectionOutput> {
    files
        .iter()
        .map(|(path, bytes)| {
            let owner = entries.iter().find(|entry| {
                let public = format!("topics/{}/{}/", entry.topic.id, entry.guide.key);
                let review = format!("review/{}/{}/", entry.topic.id, entry.guide.key);
                path.starts_with(&public) || path.starts_with(&review)
            });
            let topic = owner.map(|entry| entry.topic.id.clone()).or_else(|| {
                entries.iter().find_map(|entry| {
                    let prefix = format!("topics/{}/", entry.topic.id);
                    path.starts_with(&prefix).then(|| entry.topic.id.clone())
                })
            });
            CollectionOutput {
                kind: output_kind(path).to_owned(),
                path: path.clone(),
                size_bytes: bytes.len() as u64,
                media_type: media_type(path).to_owned(),
                sha256: journey::hex_digest(bytes),
                topic_id: topic,
                entry: owner.map(|entry| entry.guide.key.clone()),
                journey_id: owner.map(|entry| entry.journey.id.clone()),
                report_sha256: owner.and_then(|entry| entry.render_report.report_sha256.clone()),
            }
        })
        .collect()
}

fn entry_reports(entries: &[PreparedEntry]) -> Vec<CollectionEntryReport> {
    entries
        .iter()
        .map(|entry| CollectionEntryReport {
            key: entry.guide.key.clone(),
            topic_id: entry.topic.id.clone(),
            status: entry.render_report.status,
            run_id: entry.render_report.run_id.clone(),
            report_sha256: entry.render_report.report_sha256.clone(),
            journey: entry.journey.clone(),
            reason_code: entry.render_report.reason.code.clone(),
            guide_steps: entry.render_report.guide_steps,
            findings: entry.render_report.findings,
        })
        .collect()
}

fn output_kind(path: &str) -> &'static str {
    if path == "index.md" || path.ends_with("/index.md") {
        "index"
    } else if path == "guide-collection.json" {
        "collection"
    } else if path.ends_with("findings.json") {
        "findings_json"
    } else if path.ends_with("findings.md") {
        "findings_markdown"
    } else if path.ends_with(".png") {
        "focused_image"
    } else if path.ends_with(".json") {
        "evidence_json"
    } else {
        "evidence"
    }
}

fn media_type(path: &str) -> &'static str {
    if path.ends_with(".md") {
        "text/markdown"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else {
        "application/octet-stream"
    }
}

fn snapshot_digest(
    manifest_sha256: &str,
    entries: &[CollectionEntryReport],
    outputs: &[CollectionOutput],
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"crawlson-guide-collection-v1\0");
    digest.update(manifest_sha256.as_bytes());
    for entry in entries {
        digest.update(b"\0entry\0");
        digest.update(serde_json::to_vec(entry).expect("entry provenance is serializable"));
    }
    for output in outputs {
        if output.kind == "collection" {
            continue;
        }
        digest.update(b"\0output\0");
        digest.update(serde_json::to_vec(output).expect("output provenance is serializable"));
    }
    hex_digest(digest.finalize().as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn diagnostics_for_entries(
    status: CollectionStatus,
    entries: &[PreparedEntry],
) -> Vec<CollectionDiagnostic> {
    match status {
        CollectionStatus::Ready => Vec::new(),
        CollectionStatus::Findings => vec![CollectionDiagnostic {
            code: "findings_present".to_owned(),
            message: "deterministic findings prevent public collection output".to_owned(),
            entry: None,
            path: None,
        }],
        CollectionStatus::NotPublishable => vec![CollectionDiagnostic {
            code: "entries_unavailable".to_owned(),
            message: "blocked or unavailable entries prevent public collection output".to_owned(),
            entry: None,
            path: None,
        }],
        CollectionStatus::Error => entries
            .iter()
            .filter(|entry| entry.render_report.status == RenderStatus::Error)
            .map(|entry| CollectionDiagnostic {
                code: entry.render_report.reason.code.clone(),
                message: entry.render_report.reason.message.clone(),
                entry: Some(entry.guide.key.clone()),
                path: None,
            })
            .collect(),
    }
}

fn build(prepared: PreparedCollection) -> CollectionReport {
    match read_output_tree(&prepared.output_directory) {
        Ok(Some(actual)) if actual == prepared.files => prepared.report,
        Ok(Some(actual)) => conflict_report(
            prepared.report,
            compare_and_audit(&prepared.files, &actual, true),
            "output_conflict",
            "existing collection differs and was preserved",
        ),
        Ok(None) => match write_new_tree(&prepared.output_directory, &prepared.files) {
            Ok(()) => prepared.report,
            Err(error) => conflict_report(
                prepared.report,
                vec![diagnostic(error)],
                "output_write_failed",
                "collection output could not be installed",
            ),
        },
        Err(error) => conflict_report(
            prepared.report,
            vec![diagnostic(error)],
            "output_invalid",
            "existing collection output could not be verified",
        ),
    }
}

fn check(prepared: PreparedCollection) -> CollectionReport {
    match read_output_tree(&prepared.output_directory) {
        Ok(Some(actual)) if actual == prepared.files => prepared.report,
        Ok(Some(actual)) => {
            let diagnostics = compare_and_audit(&prepared.files, &actual, false);
            let structural = diagnostics.iter().any(|diagnostic| {
                matches!(
                    diagnostic.code.as_str(),
                    "dead_link"
                        | "orphan_image"
                        | "missing_index_entry"
                        | "output_symlink_rejected"
                        | "unexpected_output"
                        | "missing_output"
                        | "changed_output"
                )
            });
            let status = if structural {
                CollectionStatus::Error
            } else {
                CollectionStatus::NotPublishable
            };
            checked_mismatch_report(prepared.report, status, diagnostics)
        }
        Ok(None) => checked_mismatch_report(
            prepared.report,
            CollectionStatus::NotPublishable,
            vec![CollectionDiagnostic {
                code: "output_missing".to_owned(),
                message: "collection output does not exist".to_owned(),
                entry: None,
                path: None,
            }],
        ),
        Err(error) => checked_mismatch_report(
            prepared.report,
            CollectionStatus::Error,
            vec![diagnostic(error)],
        ),
    }
}

fn conflict_report(
    mut report: CollectionReport,
    mut diagnostics: Vec<CollectionDiagnostic>,
    code: &str,
    message: &str,
) -> CollectionReport {
    diagnostics.push(CollectionDiagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        entry: None,
        path: None,
    });
    diagnostics.sort();
    diagnostics.dedup();
    report.status = CollectionStatus::Error;
    report.publishable = false;
    report.summary.errors += 1;
    report.diagnostics = diagnostics;
    report.outputs.clear();
    report
}

fn checked_mismatch_report(
    mut report: CollectionReport,
    status: CollectionStatus,
    mut diagnostics: Vec<CollectionDiagnostic>,
) -> CollectionReport {
    diagnostics.sort();
    diagnostics.dedup();
    report.status = status;
    report.publishable = false;
    if status == CollectionStatus::Error {
        report.summary.errors += 1;
    }
    report.diagnostics = diagnostics;
    report.outputs.clear();
    report
}

fn diagnostic(error: CollectionError) -> CollectionDiagnostic {
    CollectionDiagnostic {
        code: error.code.to_owned(),
        message: error.message,
        entry: error.entry,
        path: error.path,
    }
}

fn write_new_tree(output: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), CollectionError> {
    let parent = output.parent().ok_or_else(|| {
        CollectionError::new("output_write_failed", "output directory has no parent")
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".crawlson-collection-")
        .tempdir_in(parent)
        .map_err(|_| {
            CollectionError::new(
                "output_write_failed",
                "collection staging directory could not be created",
            )
        })?;
    for (relative, bytes) in files {
        let path = staging.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                CollectionError::new(
                    "output_write_failed",
                    "collection staging directories could not be created",
                )
            })?;
        }
        atomic_write(&path, bytes)?;
    }
    let staged = read_output_tree(staging.path())?.ok_or_else(|| {
        CollectionError::new(
            "output_write_failed",
            "staged collection unexpectedly disappeared",
        )
    })?;
    if staged != *files {
        return Err(CollectionError::new(
            "output_write_failed",
            "staged collection did not reverify byte-for-byte",
        ));
    }
    let staging_path = staging.keep();
    if rename_create_only(&staging_path, output).is_err() {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(CollectionError::new(
            "output_write_failed",
            "collection could not be installed without overwriting output",
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_create_only(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both pointers come from live NUL-terminated C strings, and the
    // flags request a same-filesystem, no-replace rename.
    let result = unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            destination.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_create_only(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renamex_np(old: *const c_char, new: *const c_char, flags: c_uint) -> c_int;
    }
    const RENAME_EXCL: c_uint = 0x0000_0004;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both pointers come from live NUL-terminated C strings, and
    // RENAME_EXCL makes the kernel reject an existing destination.
    let result = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "windows")]
fn rename_create_only(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
)))]
fn rename_create_only(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "create-only directory rename is unsupported on this platform",
    ))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CollectionError> {
    let mut file = AtomicWriteFile::open(path).map_err(|_| {
        CollectionError::new(
            "output_write_failed",
            "staged collection file could not be opened",
        )
    })?;
    file.write_all(bytes).map_err(|_| {
        CollectionError::new(
            "output_write_failed",
            "staged collection file could not be written",
        )
    })?;
    file.commit().map_err(|_| {
        CollectionError::new(
            "output_write_failed",
            "staged collection file could not be committed",
        )
    })
}

fn read_output_tree(output: &Path) -> Result<Option<BTreeMap<String, Vec<u8>>>, CollectionError> {
    match fs::symlink_metadata(output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(CollectionError::new(
                "output_invalid",
                "collection output metadata is unavailable",
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(CollectionError::new(
                "output_symlink_rejected",
                "collection output must be a real directory without symlinks",
            ));
        }
        Ok(_) => {}
    }
    let mut files = BTreeMap::new();
    let mut limits = CopyLimits::default();
    read_output_directory(output, output, 0, &mut limits, &mut files)?;
    Ok(Some(files))
}

fn read_output_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    limits: &mut CopyLimits,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), CollectionError> {
    if depth > MAX_INPUT_DEPTH {
        return Err(CollectionError::new(
            "output_invalid",
            "collection output nesting exceeds its bound",
        ));
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|_| CollectionError::new("output_invalid", "output could not be enumerated"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| CollectionError::new("output_invalid", "output entry is unavailable"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| {
            CollectionError::new("output_invalid", "output entry metadata is unavailable")
        })?;
        let entry_path = entry.path();
        let relative = entry_path
            .strip_prefix(root)
            .map_err(|_| CollectionError::new("output_invalid", "output entry escaped its root"))?;
        let portable = path_to_portable(relative)?;
        if metadata.file_type().is_symlink() {
            return Err(CollectionError::new(
                "output_symlink_rejected",
                "collection output may not contain symlinks",
            )
            .path(&portable));
        }
        if metadata.is_dir() {
            read_output_directory(root, &entry_path, depth + 1, limits, files)?;
        } else if metadata.is_file() {
            limits.reserve(
                metadata.len(),
                MAX_OUTPUT_FILES,
                MAX_OUTPUT_BYTES,
                "output_invalid",
            )?;
            let bytes = read_bounded(&entry_path, MAX_OUTPUT_BYTES, "output_invalid")?;
            if files.insert(portable.clone(), bytes).is_some() {
                return Err(CollectionError::new(
                    "output_invalid",
                    "collection output contains a duplicate path",
                )
                .path(&portable));
            }
        } else {
            return Err(CollectionError::new(
                "output_invalid",
                "collection output contains an unsupported filesystem entry",
            )
            .path(&portable));
        }
    }
    Ok(())
}

fn path_to_portable(path: &Path) -> Result<String, CollectionError> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(CollectionError::new(
                "output_invalid",
                "output contains a non-portable path",
            ));
        };
        let part = part.to_str().ok_or_else(|| {
            CollectionError::new("output_invalid", "output contains a non-UTF-8 path")
        })?;
        if !portable_segment(part) {
            return Err(CollectionError::new(
                "output_invalid",
                "output contains a non-portable path",
            ));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn compare_and_audit(
    expected: &BTreeMap<String, Vec<u8>>,
    actual: &BTreeMap<String, Vec<u8>>,
    conflict: bool,
) -> Vec<CollectionDiagnostic> {
    let mut diagnostics = audit_tree(actual);
    let stale_inputs = !conflict && collection_identity_changed(expected, actual);
    for path in expected.keys() {
        match actual.get(path) {
            None => diagnostics.push(CollectionDiagnostic {
                code: if stale_inputs {
                    "stale_output".to_owned()
                } else {
                    "missing_output".to_owned()
                },
                message: if stale_inputs {
                    "generated file set no longer matches current inputs".to_owned()
                } else {
                    "expected generated file is missing".to_owned()
                },
                entry: None,
                path: Some(path.clone()),
            }),
            Some(bytes) if bytes != &expected[path] => diagnostics.push(CollectionDiagnostic {
                code: if stale_inputs {
                    "stale_output".to_owned()
                } else {
                    "changed_output".to_owned()
                },
                message: if stale_inputs {
                    "generated file no longer matches current inputs".to_owned()
                } else {
                    "existing generated file differs".to_owned()
                },
                entry: None,
                path: Some(path.clone()),
            }),
            Some(_) => {}
        }
    }
    for path in actual.keys().filter(|path| !expected.contains_key(*path)) {
        diagnostics.push(CollectionDiagnostic {
            code: if stale_inputs {
                "stale_output".to_owned()
            } else {
                "unexpected_output".to_owned()
            },
            message: if stale_inputs {
                "generated file set no longer matches current inputs".to_owned()
            } else {
                "unregistered file is present in the generated collection".to_owned()
            },
            entry: None,
            path: Some(path.clone()),
        });
    }
    diagnostics.sort();
    diagnostics.dedup();
    diagnostics
}

fn collection_identity_changed(
    expected: &BTreeMap<String, Vec<u8>>,
    actual: &BTreeMap<String, Vec<u8>>,
) -> bool {
    fn identity(files: &BTreeMap<String, Vec<u8>>) -> Option<(String, String)> {
        let report: serde_json::Value =
            serde_json::from_slice(files.get("collection-report.json")?).ok()?;
        let collection = report.get("collection")?;
        Some((
            collection.get("manifest_sha256")?.as_str()?.to_owned(),
            collection.get("snapshot_sha256")?.as_str()?.to_owned(),
        ))
    }
    match (identity(expected), identity(actual)) {
        (Some(expected), Some(actual)) => expected != actual,
        _ => false,
    }
}

fn audit_generated_tree(files: &BTreeMap<String, Vec<u8>>) -> Result<(), CollectionError> {
    let diagnostics = audit_tree(files);
    if let Some(diagnostic) = diagnostics.first() {
        return Err(CollectionError::new(
            "generated_links_invalid",
            "generated collection failed its internal link and reachability audit",
        )
        .path(diagnostic.path.as_deref().unwrap_or("collection")));
    }
    Ok(())
}

fn audit_tree(files: &BTreeMap<String, Vec<u8>>) -> Vec<CollectionDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut referenced_images = BTreeSet::new();
    for (path, bytes) in files.iter().filter(|(path, _)| path.ends_with(".md")) {
        let Ok(markdown) = std::str::from_utf8(bytes) else {
            diagnostics.push(CollectionDiagnostic {
                code: "markdown_invalid".to_owned(),
                message: "generated Markdown is not UTF-8".to_owned(),
                entry: None,
                path: Some(path.clone()),
            });
            continue;
        };
        for link in markdown_links(markdown) {
            let Some(target) = resolve_markdown_target(path, &link.target) else {
                diagnostics.push(CollectionDiagnostic {
                    code: "dead_link".to_owned(),
                    message: "Markdown contains an unsafe or unsupported local link".to_owned(),
                    entry: None,
                    path: Some(path.clone()),
                });
                continue;
            };
            if !files.contains_key(&target) {
                diagnostics.push(CollectionDiagnostic {
                    code: "dead_link".to_owned(),
                    message: "Markdown link target is missing".to_owned(),
                    entry: None,
                    path: Some(target),
                });
                continue;
            }
            if link.image || target.ends_with(".png") {
                referenced_images.insert(target);
            } else if target.ends_with(".md") {
                graph.entry(path.clone()).or_default().insert(target);
            }
        }
    }
    for path in files.keys().filter(|path| path.ends_with(".png")) {
        if !referenced_images.contains(path) {
            diagnostics.push(CollectionDiagnostic {
                code: "orphan_image".to_owned(),
                message: "generated image is not referenced by Markdown".to_owned(),
                entry: None,
                path: Some(path.clone()),
            });
        }
    }
    let roots = ["index.md", "review/index.md"]
        .into_iter()
        .filter(|root| files.contains_key(*root))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut reachable = BTreeSet::new();
    let mut queue = VecDeque::from(roots);
    while let Some(path) = queue.pop_front() {
        if !reachable.insert(path.clone()) {
            continue;
        }
        if let Some(targets) = graph.get(&path) {
            queue.extend(targets.iter().cloned());
        }
    }
    for path in files.keys().filter(|path| path.ends_with(".md")) {
        if !reachable.contains(path) {
            diagnostics.push(CollectionDiagnostic {
                code: "missing_index_entry".to_owned(),
                message: "generated Markdown page is not reachable from its collection index"
                    .to_owned(),
                entry: None,
                path: Some(path.clone()),
            });
        }
    }
    diagnostics.sort();
    diagnostics.dedup();
    diagnostics
}

#[derive(Debug)]
struct MarkdownLink {
    image: bool,
    target: String,
}

fn markdown_links(markdown: &str) -> Vec<MarkdownLink> {
    let bytes = markdown.as_bytes();
    let mut links = Vec::new();
    let mut index = 0usize;
    let mut fenced = false;
    let mut inline_ticks = 0usize;
    while index + 2 < bytes.len() {
        if bytes[index] == b'`' && !byte_is_escaped(bytes, index) {
            let ticks = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            let line_prefix_is_space = markdown[..index].rsplit_once('\n').map_or_else(
                || markdown[..index].trim().is_empty(),
                |(_, prefix)| prefix.trim().is_empty(),
            );
            if ticks >= 3 && line_prefix_is_space && inline_ticks == 0 {
                fenced = !fenced;
                index += ticks;
                continue;
            }
            if !fenced {
                if inline_ticks == 0 {
                    inline_ticks = ticks;
                } else if inline_ticks == ticks {
                    inline_ticks = 0;
                }
            }
            index += ticks;
            continue;
        }
        if fenced || inline_ticks > 0 {
            index += 1;
            continue;
        }
        if byte_is_escaped(bytes, index) {
            index += 1;
            continue;
        }
        let image = bytes[index] == b'!' && bytes.get(index + 1) == Some(&b'[');
        let bracket = if image {
            index + 1
        } else if bytes[index] == b'[' {
            index
        } else {
            index += 1;
            continue;
        };
        let Some(close) = markdown[bracket + 1..].find("](") else {
            break;
        };
        let target_start = bracket + 1 + close + 2;
        let Some(close_paren) = markdown[target_start..].find(')') else {
            break;
        };
        let target_end = target_start + close_paren;
        links.push(MarkdownLink {
            image,
            target: markdown[target_start..target_end].to_owned(),
        });
        index = target_end + 1;
    }
    links
}

fn byte_is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}

fn resolve_markdown_target(source: &str, target: &str) -> Option<String> {
    if target.is_empty()
        || target.starts_with('/')
        || target.starts_with('#')
        || target.contains("//")
        || target.contains('\\')
        || target.contains('?')
        || target.contains('#')
        || target.chars().any(char::is_control)
    {
        return None;
    }
    let mut parts = source.split('/').collect::<Vec<_>>();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => return None,
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_limits_fail_before_reserving_excess_bytes() {
        let mut limits = CopyLimits::default();
        limits.reserve(4, 2, 8, "bounded").unwrap();
        limits.reserve(4, 2, 8, "bounded").unwrap();
        assert!(limits.reserve(1, 2, 8, "bounded").is_err());
        assert_eq!(limits.files, 2);
        assert_eq!(limits.bytes, 8);
    }

    #[test]
    fn portable_paths_reject_aliasing_and_windows_device_segments() {
        for path in [
            "runs//one",
            "runs/one/",
            "runs/con/report.json",
            "runs/COM1.txt/report.json",
            "runs/trailing./report.json",
            "runs/question?/report.json",
        ] {
            assert!(validate_portable_relative(path, "path").is_err(), "{path}");
        }
        assert!(validate_portable_relative("runs/one/report.json", "path").is_ok());
    }

    #[test]
    fn markdown_links_ignore_inline_and_fenced_code() {
        let markdown = "`[not a link](missing.md)`\n\n```text\n![also not](missing.png)\n```\n\n[real](present.md)\n";
        let links = markdown_links(markdown);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "present.md");
        assert!(!links[0].image);
    }

    #[test]
    fn create_only_rename_preserves_a_concurrent_destination() {
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("source");
        let destination = parent.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("source.txt"), b"source").unwrap();
        fs::create_dir(&destination).unwrap();

        assert!(rename_create_only(&source, &destination).is_err());
        assert!(destination.is_dir());
        assert!(fs::read_dir(&destination).unwrap().next().is_none());
        assert_eq!(fs::read(source.join("source.txt")).unwrap(), b"source");
    }
}
