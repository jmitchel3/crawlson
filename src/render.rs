use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::focus::{self, PngInspection};
use crate::journey::{self, ValidatedAction, ValidatedJourney};
use crate::{CommandResult, VERSION};

const MAX_REPORT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SCREENSHOT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FOCUS_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_TRACE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARTIFACT_TOTAL_BYTES: u64 = 384 * 1024 * 1024;

pub const EXIT_FINDINGS_READY: u8 = 1;
pub const EXIT_NOT_PUBLISHABLE: u8 = 3;
pub const EXIT_RENDER_ERROR: u8 = 4;

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub run_directory: PathBuf,
    pub journey_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderStatus {
    GuideReady,
    FindingsReady,
    NotPublishable,
    Error,
}

impl RenderStatus {
    fn exit_code(self) -> u8 {
        match self {
            Self::GuideReady => 0,
            Self::FindingsReady => EXIT_FINDINGS_READY,
            Self::NotPublishable => EXIT_NOT_PUBLISHABLE,
            Self::Error => EXIT_RENDER_ERROR,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::GuideReady => "guide_ready",
            Self::FindingsReady => "findings_ready",
            Self::NotPublishable => "not_publishable",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderReport {
    pub schema_version: u8,
    pub crawlson_version: &'static str,
    pub status: RenderStatus,
    pub publishable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journey: Option<RenderedJourney>,
    pub reason: RenderReason,
    pub guide_steps: u32,
    pub findings: u32,
    pub outputs: Vec<RenderedOutput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderedJourney {
    pub id: String,
    pub revision: u32,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderedOutput {
    pub kind: &'static str,
    pub path: String,
    pub size_bytes: u64,
    pub media_type: &'static str,
    pub sha256: String,
}

impl RenderReport {
    pub fn render(&self, json: bool) -> CommandResult {
        let exit_code = self.status.exit_code();
        if json {
            let mut stdout = serde_json::to_string(self).expect("render report is serializable");
            stdout.push('\n');
            CommandResult {
                exit_code,
                stdout,
                stderr: String::new(),
            }
        } else {
            let mut stdout = format!(
                "Crawlson render: {}\nReason: {}\n",
                self.status.as_str(),
                self.reason.message
            );
            for output in &self.outputs {
                stdout.push_str(&format!("{}: {}\n", output.kind, output.path));
            }
            CommandResult {
                exit_code,
                stdout,
                stderr: String::new(),
            }
        }
    }
}

pub fn run(options: RenderOptions) -> RenderReport {
    match render_verified(options) {
        Ok(report) => report,
        Err(error) => error_report(error.code, error.message),
    }
}

#[derive(Debug)]
struct RenderError {
    code: &'static str,
    message: String,
}

impl RenderError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn error_report(code: &'static str, message: String) -> RenderReport {
    RenderReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status: RenderStatus::Error,
        publishable: false,
        run_id: None,
        report_sha256: None,
        journey: None,
        reason: RenderReason {
            code: code.to_owned(),
            message,
        },
        guide_steps: 0,
        findings: 0,
        outputs: Vec::new(),
    }
}

fn render_verified(options: RenderOptions) -> Result<RenderReport, RenderError> {
    let root = verify_run_root(&options.run_directory)?;
    let report_path = root.join("report.json");
    let report_bytes = read_regular_bounded(&report_path, MAX_REPORT_BYTES, "run report")?;
    let report_sha256 = journey::hex_digest(&report_bytes);
    let report: InputRunReport = serde_json::from_slice(&report_bytes).map_err(|error| {
        RenderError::new(
            "report_invalid",
            format!("run report v1 is invalid: {error}"),
        )
    })?;
    validate_report_header(&report)?;

    let loaded = journey::load(&options.journey_path)
        .map_err(|error| RenderError::new("journey_invalid", error.to_string()))?;
    let journey = journey::validate(loaded)
        .map_err(|error| RenderError::new("journey_invalid", error.to_string()))?;
    validate_provenance(&report, &journey)?;
    validate_renderable_journey(&journey)?;
    validate_steps(&report, &journey)?;
    let artifacts = verify_artifacts(&root, &report, &journey)?;

    let rendered_journey = RenderedJourney {
        id: journey.meta.id.clone(),
        revision: journey.meta.revision,
        source_sha256: journey.source_sha256.clone(),
    };

    if report.cleanup.status == CleanupStatus::Failed
        || (matches!(report.outcome, Outcome::Passed | Outcome::Failed)
            && (report.outcome != report.execution_outcome
                || !report.cleanup.attempted
                || report.cleanup.status != CleanupStatus::Passed))
    {
        let reason = if report.cleanup.status == CleanupStatus::Failed {
            (
                "cleanup_failed",
                "run cleanup failed; this run cannot be published",
            )
        } else {
            (
                "run_incomplete",
                "final and execution outcomes or cleanup state are incomplete",
            )
        };
        return publish(
            &root,
            RenderReport {
                schema_version: 1,
                crawlson_version: VERSION,
                status: RenderStatus::Error,
                publishable: false,
                run_id: Some(report.run_id),
                report_sha256: Some(report_sha256),
                journey: Some(rendered_journey),
                reason: RenderReason {
                    code: reason.0.to_owned(),
                    message: reason.1.to_owned(),
                },
                guide_steps: 0,
                findings: 0,
                outputs: Vec::new(),
            },
            Vec::new(),
        );
    }

    match report.outcome {
        Outcome::Passed => build_guide(
            &root,
            report,
            &journey,
            &artifacts,
            rendered_journey,
            report_sha256,
        ),
        Outcome::Failed => build_findings(
            &root,
            report,
            &journey,
            &artifacts,
            rendered_journey,
            report_sha256,
        ),
        Outcome::Blocked => publish(
            &root,
            RenderReport {
                schema_version: 1,
                crawlson_version: VERSION,
                status: RenderStatus::NotPublishable,
                publishable: false,
                run_id: Some(report.run_id),
                report_sha256: Some(report_sha256),
                journey: Some(rendered_journey),
                reason: RenderReason {
                    code: "run_blocked".to_owned(),
                    message: "blocked runs cannot produce a guide or finding".to_owned(),
                },
                guide_steps: 0,
                findings: 0,
                outputs: Vec::new(),
            },
            Vec::new(),
        ),
        Outcome::Error => publish(
            &root,
            RenderReport {
                schema_version: 1,
                crawlson_version: VERSION,
                status: RenderStatus::Error,
                publishable: false,
                run_id: Some(report.run_id),
                report_sha256: Some(report_sha256),
                journey: Some(rendered_journey),
                reason: RenderReason {
                    code: "run_error".to_owned(),
                    message: "error runs cannot produce a guide or finding".to_owned(),
                },
                guide_steps: 0,
                findings: 0,
                outputs: Vec::new(),
            },
            Vec::new(),
        ),
    }
}

fn verify_run_root(path: &Path) -> Result<PathBuf, RenderError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| RenderError::new("run_directory_invalid", error.to_string()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(RenderError::new(
            "run_directory_invalid",
            "run directory must be a real directory, not a symlink",
        ));
    }
    path.canonicalize()
        .map_err(|error| RenderError::new("run_directory_invalid", error.to_string()))
}

fn read_regular_bounded(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>, RenderError> {
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| RenderError::new("artifact_invalid", format!("{label}: {error}")))?;
    if !path_metadata.file_type().is_file() || path_metadata.file_type().is_symlink() {
        return Err(RenderError::new(
            "artifact_invalid",
            format!("{label} must be a regular file, not a symlink"),
        ));
    }
    let file = fs::File::open(path)
        .map_err(|error| RenderError::new("artifact_invalid", format!("{label}: {error}")))?;
    let metadata = file
        .metadata()
        .map_err(|error| RenderError::new("artifact_invalid", format!("{label}: {error}")))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(RenderError::new(
            "artifact_invalid",
            format!("{label} must contain 1 to {maximum} bytes"),
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| RenderError::new("artifact_invalid", format!("{label}: {error}")))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(RenderError::new(
            "artifact_invalid",
            format!("{label} changed while it was being read"),
        ));
    }
    Ok(bytes)
}

fn validate_report_header(report: &InputRunReport) -> Result<(), RenderError> {
    if !matches!(report.schema_version, 1..=3) {
        return Err(RenderError::new(
            "report_version_unsupported",
            "only run report schema versions 1, 2, and 3 can be rendered",
        ));
    }
    if report.run_directory.len() > 32_768 || report.run_directory.chars().any(char::is_control) {
        return Err(RenderError::new(
            "report_invalid",
            "reported run directory metadata is invalid",
        ));
    }
    if report.run_id.is_empty()
        || report.run_id.len() > 128
        || !report
            .run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(RenderError::new("report_invalid", "run_id is invalid"));
    }
    let _wall_clock_metadata = (report.started_at_unix_ms, report.finished_at_unix_ms);
    if report.driver.name != "agent-browser" {
        return Err(RenderError::new(
            "report_invalid",
            "run report driver must be agent-browser for schema version 1",
        ));
    }
    if semver::Version::parse(&report.crawlson_version).is_err()
        || !valid_reason(&report.reason)
        || !valid_reason(&report.execution_reason)
        || report.cleanup.error.as_deref().is_some_and(str::is_empty)
    {
        return Err(RenderError::new(
            "report_invalid",
            "run report version, reason, or cleanup metadata is invalid",
        ));
    }
    if let Some(diagnostics) = &report.diagnostics
        && (diagnostics.console_messages > u32::MAX as u64
            || diagnostics.page_errors > u32::MAX as u64
            || !is_sha256(&diagnostics.console_sha256)
            || !is_sha256(&diagnostics.page_errors_sha256))
    {
        return Err(RenderError::new(
            "report_invalid",
            "diagnostic digests are invalid",
        ));
    }
    validate_driver(&report.driver)?;
    validate_outcome_contract(report)?;
    Ok(())
}

fn validate_outcome_contract(report: &InputRunReport) -> Result<(), RenderError> {
    let cleanup_valid = match report.cleanup.status {
        CleanupStatus::NotNeeded => !report.cleanup.attempted && report.cleanup.error.is_none(),
        CleanupStatus::Passed => report.cleanup.attempted && report.cleanup.error.is_none(),
        CleanupStatus::Failed => report.cleanup.attempted && report.cleanup.error.is_some(),
    };
    let terminal_valid = match report.outcome {
        Outcome::Passed => {
            report.execution_outcome == Outcome::Passed
                && report.reason.code == "journey_passed"
                && report.execution_reason.code == "journey_passed"
                && report.reason == report.execution_reason
        }
        Outcome::Failed => {
            report.execution_outcome == Outcome::Failed
                && report.reason.code == "checkpoint_failed"
                && report.execution_reason.code == "checkpoint_failed"
                && report.reason == report.execution_reason
        }
        Outcome::Blocked => {
            report.execution_outcome == Outcome::Blocked && report.reason == report.execution_reason
        }
        Outcome::Error => true,
    };
    if !cleanup_valid || !terminal_valid {
        return Err(RenderError::new(
            "report_invalid",
            "run outcome, reason, or cleanup fields are contradictory",
        ));
    }
    Ok(())
}

fn valid_reason(reason: &InputReason) -> bool {
    !reason.code.is_empty()
        && reason
            .code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !reason.message.trim().is_empty()
}

fn validate_driver(driver: &InputDriver) -> Result<(), RenderError> {
    if driver
        .version
        .as_deref()
        .is_some_and(|version| semver::Version::parse(version).is_err())
        || driver.session.as_deref().is_some_and(|session| {
            session.is_empty()
                || session.len() > 48
                || !session.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                })
        })
    {
        return Err(RenderError::new(
            "report_invalid",
            "driver version or session provenance is invalid",
        ));
    }
    let mut expected = 1;
    const ALLOWED: &[&str] = &[
        "set_viewport",
        "authentication_load",
        "trace_start",
        "navigate",
        "current_url",
        "text",
        "is_visible",
        "is_enabled",
        "get_attribute",
        "click",
        "bounding_box",
        "screenshot",
        "console",
        "page_errors",
        "trace_stop",
        "close",
    ];
    for command in &driver.commands {
        let authentication_output_redacted = command.capability != "authentication_load"
            || (command.stdout_bytes == 0
                && command.stdout_captured_bytes == 0
                && command.stdout_captured_sha256 == journey::hex_digest(&[])
                && command.stderr_bytes == 0
                && command.stderr_captured_bytes == 0
                && command.stderr_captured_sha256 == journey::hex_digest(&[]));
        if command.sequence != expected
            || command.capability.trim().is_empty()
            || !ALLOWED.contains(&command.capability.as_str())
            || command.stdout_captured_bytes > command.stdout_bytes
            || command.stderr_captured_bytes > command.stderr_bytes
            || !is_sha256(&command.stdout_captured_sha256)
            || !is_sha256(&command.stderr_captured_sha256)
            || command.duration_ms > 3_600_000
            || (command.upstream_success && command.exit_code != Some(0))
            || !authentication_output_redacted
        {
            return Err(RenderError::new(
                "report_invalid",
                "driver command provenance is invalid",
            ));
        }
        expected += 1;
    }
    Ok(())
}

fn validate_completed_driver(
    report: &InputRunReport,
    journey: &ValidatedJourney,
) -> Result<(), RenderError> {
    let version = report
        .driver
        .version
        .as_deref()
        .and_then(|value| semver::Version::parse(value).ok())
        .filter(|version| version.major == 0 && version.minor == 26 && version.pre.is_empty());
    let session = report.driver.session.as_deref();
    let commands = &report.driver.commands;
    let authentication_offset = usize::from(journey.schema_version == 4);
    let lifecycle_valid = commands.len() >= 6 + authentication_offset
        && commands[0].capability == "set_viewport"
        && (authentication_offset == 0 || commands[1].capability == "authentication_load")
        && commands[1 + authentication_offset].capability == "trace_start"
        && commands[commands.len() - 4].capability == "console"
        && commands[commands.len() - 3].capability == "page_errors"
        && commands[commands.len() - 2].capability == "trace_stop"
        && commands[commands.len() - 1].capability == "close";
    if version.is_none()
        || session.is_none()
        || !lifecycle_valid
        || commands
            .iter()
            .any(|command| !command.upstream_success || command.exit_code != Some(0))
    {
        return Err(RenderError::new(
            "report_invalid",
            "completed execution driver provenance is incomplete or unsuccessful",
        ));
    }

    let count = |capability: &str| {
        commands
            .iter()
            .filter(|command| command.capability == capability)
            .count()
    };
    let executed_pairs = journey.steps.iter().zip(&report.steps).collect::<Vec<_>>();
    let navigate_count = executed_pairs
        .iter()
        .filter(|(declared, _)| matches!(declared.action, ValidatedAction::Navigate { .. }))
        .count();
    let visible_count = report
        .steps
        .iter()
        .filter(|step| step.observation.visible.is_some())
        .count();
    let text_count = executed_pairs
        .iter()
        .filter(|(declared, executed)| {
            matches!(declared.action, ValidatedAction::CheckText { .. })
                && executed.observation.visible == Some(true)
        })
        .count();
    let enabled_count = report
        .steps
        .iter()
        .filter(|step| step.observation.enabled.is_some())
        .count();
    let attribute_count = report
        .steps
        .iter()
        .filter(|step| step.kind == "follow_link" && step.observation.enabled == Some(true))
        .count();
    let capture_count = report
        .steps
        .iter()
        .filter(|step| step.observation.box_command_sequence.is_some())
        .count();
    let click_count = report
        .steps
        .iter()
        .filter(|step| step.observation.action_command_sequence.is_some())
        .count();
    if count("set_viewport") != 1
        || count("authentication_load") != authentication_offset
        || count("trace_start") != 1
        || count("navigate") != navigate_count
        || count("is_visible") != visible_count
        || count("is_enabled") != enabled_count
        || count("get_attribute") != attribute_count
        || count("click") != click_count
        || count("text") != text_count
        || count("bounding_box") != capture_count
        || count("screenshot") != capture_count
        || count("console") != 1
        || count("page_errors") != 1
        || count("trace_stop") != 1
        || count("close") != 1
        || count("current_url") < report.steps.len()
    {
        return Err(RenderError::new(
            "report_invalid",
            "completed execution commands do not match the journey lifecycle",
        ));
    }

    let session = session.expect("session presence was checked");
    for step in report
        .steps
        .iter()
        .filter(|step| step.observation.box_command_sequence.is_some())
    {
        let box_sequence = step.observation.box_command_sequence.unwrap_or(0);
        let screenshot_sequence = step.observation.screenshot_command_sequence.unwrap_or(0);
        let box_command = commands.get(box_sequence.saturating_sub(1) as usize);
        let screenshot_command = commands.get(screenshot_sequence.saturating_sub(1) as usize);
        let action_sequence = step.observation.action_command_sequence;
        let action_command =
            action_sequence.and_then(|sequence| commands.get(sequence.saturating_sub(1) as usize));
        let expected_token = action_sequence.map_or_else(
            || format!("{session}:{box_sequence}:{screenshot_sequence}"),
            |sequence| format!("{session}:{box_sequence}:{screenshot_sequence}:{sequence}"),
        );
        if box_command.is_none_or(|command| command.capability != "bounding_box")
            || screenshot_command.is_none_or(|command| command.capability != "screenshot")
            || action_sequence.is_some()
                && action_command.is_none_or(|command| command.capability != "click")
            || action_sequence
                .is_some_and(|sequence| sequence != screenshot_sequence.saturating_add(1))
            || step.observation.capture_token.as_deref() != Some(expected_token.as_str())
        {
            return Err(RenderError::new(
                "report_invalid",
                "capture command provenance does not match the driver command sequence",
            ));
        }
    }
    Ok(())
}

fn validate_provenance(
    report: &InputRunReport,
    journey: &ValidatedJourney,
) -> Result<(), RenderError> {
    let expected_report_version = match journey.schema_version {
        4 => 3,
        3 => 2,
        _ => 1,
    };
    if report.schema_version != expected_report_version
        || report.journey.source_path.is_empty()
        || report.journey.source_path.contains(['/', '\\'])
        || report.journey.source_sha256.as_deref() != Some(&journey.source_sha256)
        || report.journey.id.as_deref() != Some(&journey.meta.id)
        || report.journey.revision != Some(journey.meta.revision)
        || report.target_origin.as_deref() != Some(&journey.origin.to_string())
    {
        return Err(RenderError::new(
            "journey_drift",
            "journey source, digest, identity, revision, or target differs from the run",
        ));
    }
    validate_action_authorization(report, journey)?;
    validate_authentication(report, journey)?;
    Ok(())
}

fn validate_authentication(
    report: &InputRunReport,
    journey: &ValidatedJourney,
) -> Result<(), RenderError> {
    if journey.schema_version != 4 {
        if report.authentication.is_some() {
            return Err(RenderError::new(
                "report_invalid",
                "legacy run report unexpectedly contains authentication provenance",
            ));
        }
        return Ok(());
    }
    let declared = journey
        .authentication
        .as_ref()
        .expect("journey v4 authentication was validated");
    let verification_step = declared
        .verification_step
        .as_deref()
        .expect("journey v4 verification_step was validated");
    let authentication = report.authentication.as_ref().ok_or_else(|| {
        RenderError::new(
            "report_invalid",
            "authenticated report omitted authentication provenance",
        )
    })?;
    let binding = format!(
        "crawlson-auth-requirement-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nprovider={}\nrole={}\nverification_step={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        declared.provider,
        declared.role,
        verification_step
    );
    let reason_status = |code: &str| match code {
        "authentication_state_missing" => Some(InputAuthenticationStatus::Missing),
        "authentication_provider_unsupported" => Some(InputAuthenticationStatus::Unsupported),
        "authentication_state_invalid" => Some(InputAuthenticationStatus::Invalid),
        "authentication_state_load_failed" => Some(InputAuthenticationStatus::LoadFailed),
        "authentication_verification_failed" => Some(InputAuthenticationStatus::Blocked),
        _ => None,
    };
    let verification_passed = report
        .steps
        .iter()
        .any(|step| step.id == verification_step && step.status == Outcome::Passed);
    let expected_status = if verification_passed {
        InputAuthenticationStatus::Verified
    } else {
        reason_status(&report.execution_reason.code).unwrap_or(InputAuthenticationStatus::Blocked)
    };
    let load_commands = report
        .driver
        .commands
        .iter()
        .enumerate()
        .filter(|(_, command)| command.capability == "authentication_load")
        .collect::<Vec<_>>();
    let successful_load = load_commands.first().is_some_and(|(index, command)| {
        *index == 1 && command.upstream_success && command.exit_code == Some(0)
    });
    let load_lifecycle_valid = load_commands.len() <= 1
        && load_commands.first().is_none_or(|(index, _)| *index == 1)
        && (report.steps.is_empty() && !verification_passed || successful_load)
        && (!matches!(
            authentication.status,
            InputAuthenticationStatus::Missing
                | InputAuthenticationStatus::Unsupported
                | InputAuthenticationStatus::Invalid
        ) || report.driver.commands.is_empty())
        && (authentication.status != InputAuthenticationStatus::LoadFailed
            || report.steps.is_empty());
    if authentication.provider != declared.provider
        || authentication.role != declared.role
        || authentication.verification_step != verification_step
        || authentication.binding_sha256 != journey::hex_digest(binding.as_bytes())
        || authentication.status != expected_status
        || reason_status(&report.reason.code).is_some_and(|status| status != authentication.status)
        || !load_lifecycle_valid
    {
        return Err(RenderError::new(
            "report_invalid",
            "authentication provenance does not match the journey and run outcome",
        ));
    }
    Ok(())
}

fn validate_action_authorization(
    report: &InputRunReport,
    journey: &ValidatedJourney,
) -> Result<(), RenderError> {
    let mut required = journey
        .steps
        .iter()
        .filter(|step| matches!(step.action, ValidatedAction::FollowLink { .. }))
        .map(|step| format!("{}@{}:{}", journey.meta.id, journey.meta.revision, step.id))
        .collect::<Vec<_>>();
    required.sort();
    if required.is_empty() && journey.schema_version < 3 {
        if report.action_authorization.is_some() {
            return Err(RenderError::new(
                "report_invalid",
                "read-only report unexpectedly contains action authorization",
            ));
        }
        return Ok(());
    }
    let authorization = report.action_authorization.as_ref().ok_or_else(|| {
        RenderError::new(
            "report_invalid",
            "interactive report omitted action authorization provenance",
        )
    })?;
    let mut sorted_granted = authorization.granted.clone();
    sorted_granted.sort();
    sorted_granted.dedup();
    if authorization.required != required
        || authorization.granted != sorted_granted
        || !is_sha256(&authorization.binding_sha256)
        || (!report.driver.commands.is_empty() && authorization.granted != required)
    {
        return Err(RenderError::new(
            "report_invalid",
            "action authorization grants do not match the journey",
        ));
    }
    let binding = format!(
        "crawlson-action-grant-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nrequired={}\ngranted={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        required.join(","),
        authorization.granted.join(",")
    );
    if authorization.binding_sha256 != journey::hex_digest(binding.as_bytes()) {
        return Err(RenderError::new(
            "report_invalid",
            "action authorization binding digest is invalid",
        ));
    }
    Ok(())
}

fn validate_renderable_journey(journey: &ValidatedJourney) -> Result<(), RenderError> {
    let meta_safe = safe_single_line(&journey.meta.title, 4_096)
        && safe_prose(&journey.meta.purpose, 65_536)
        && safe_prose(&journey.meta.expected_outcome, 65_536);
    let steps_safe = journey.steps.iter().all(|step| {
        safe_single_line(&step.title, 4_096)
            && step
                .guide_instruction
                .as_deref()
                .is_none_or(|value| safe_prose(value, 16_384))
            && match &step.action {
                ValidatedAction::Navigate { url } | ValidatedAction::CheckUrl { url } => {
                    url.query().is_none() && url.fragment().is_none()
                }
                ValidatedAction::CheckText {
                    selector, expected, ..
                } => safe_single_line(selector, 4_096) && safe_prose(expected, 65_536),
                ValidatedAction::Capture { selector, alt_text } => {
                    safe_single_line(selector, 4_096) && safe_prose(alt_text, 4_096)
                }
                ValidatedAction::FollowLink {
                    selector,
                    expected_url,
                    alt_text,
                } => {
                    safe_single_line(selector, 4_096)
                        && safe_prose(alt_text, 4_096)
                        && expected_url.query().is_none()
                        && expected_url.fragment().is_none()
                }
            }
    });
    if !meta_safe || !steps_safe {
        return Err(RenderError::new(
            "journey_content_unrenderable",
            "journey text or URL content is unsafe or ambiguous for deterministic rendering",
        ));
    }
    Ok(())
}

fn safe_single_line(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum
        && !value
            .chars()
            .any(|character| character.is_control() || is_bidi_control(character))
}

fn safe_prose(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum
        && !value.chars().any(|character| {
            (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
                || is_bidi_control(character)
        })
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn validate_steps(report: &InputRunReport, journey: &ValidatedJourney) -> Result<(), RenderError> {
    if matches!(report.execution_outcome, Outcome::Passed | Outcome::Failed)
        && report.diagnostics.is_none()
    {
        return Err(RenderError::new(
            "run_incomplete",
            "a completed pass or checkpoint failure requires diagnostics",
        ));
    }
    let stopped_at_failed_action = report.execution_outcome == Outcome::Failed
        && report.steps.len() < journey.steps.len()
        && report
            .steps
            .last()
            .is_some_and(|step| step.status == Outcome::Failed && step.kind == "follow_link");
    if matches!(report.execution_outcome, Outcome::Passed | Outcome::Failed)
        && report.steps.len() != journey.steps.len()
        && !stopped_at_failed_action
    {
        return Err(RenderError::new(
            "run_incomplete",
            "a completed pass or checkpoint failure must record every declared step",
        ));
    }
    if report.steps.len() > journey.steps.len() {
        return Err(RenderError::new(
            "report_invalid",
            "run contains more steps than the journey",
        ));
    }
    for (index, step) in report.steps.iter().enumerate() {
        let declared = &journey.steps[index];
        if step.sequence != index as u32 + 1
            || step.id != declared.id
            || step.title != declared.title
            || step.kind != action_kind(&declared.action)
            || step.duration_ms > report.duration_ms
        {
            return Err(RenderError::new(
                "journey_drift",
                "executed step order or identity differs from the journey",
            ));
        }
        let _step_wall_clock_metadata = step.started_at_unix_ms;
        validate_observation(step, &declared.action, journey)?;
    }
    if report.outcome == Outcome::Passed
        && report
            .steps
            .iter()
            .any(|step| step.status != Outcome::Passed)
    {
        return Err(RenderError::new(
            "report_invalid",
            "a passed run contains a non-passing step",
        ));
    }
    if report.execution_outcome == Outcome::Failed
        && !report
            .steps
            .iter()
            .any(|step| step.status == Outcome::Failed)
    {
        return Err(RenderError::new(
            "report_invalid",
            "a failed execution has no failed step",
        ));
    }
    if report.execution_outcome == Outcome::Failed
        && report.execution_reason.code != "checkpoint_failed"
    {
        return Err(RenderError::new(
            "report_invalid",
            "a failed execution must use the checkpoint_failed reason",
        ));
    }
    if report.outcome == report.execution_outcome
        && matches!(report.execution_outcome, Outcome::Passed | Outcome::Failed)
    {
        validate_completed_driver(report, journey)?;
    }
    Ok(())
}

fn validate_observation(
    step: &InputStep,
    action: &ValidatedAction,
    journey: &ValidatedJourney,
) -> Result<(), RenderError> {
    let origin = &journey.origin;
    let observation = &step.observation;
    let expected_action_binding = matches!(action, ValidatedAction::FollowLink { .. }).then(|| {
        let grant = format!(
            "{}@{}:{}",
            journey.meta.id, journey.meta.revision, step.id
        );
        let binding = format!(
            "crawlson-action-step-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nstep={}\ngrant={}\n",
            journey.meta.id,
            journey.meta.revision,
            journey.source_sha256,
            journey.origin,
            step.id,
            grant
        );
        journey::hex_digest(binding.as_bytes())
    });
    if observation
        .observed_text_sha256
        .as_deref()
        .is_some_and(|value| !is_sha256(value))
        || observation
            .action_grant_sha256
            .as_deref()
            .is_some_and(|value| !is_sha256(value))
    {
        return Err(RenderError::new(
            "report_invalid",
            "step text digest is invalid",
        ));
    }
    if observation.detail.as_deref().is_some_and(|detail| {
        detail.trim().is_empty() || detail.len() > 65_536 || detail.chars().any(char::is_control)
    }) || observation
        .expected_url
        .as_deref()
        .is_some_and(|value| url::Url::parse(value).is_err())
        || observation
            .observed_url
            .as_deref()
            .is_some_and(|value| value != "unauthorized-origin" && url::Url::parse(value).is_err())
        || observation
            .before_url
            .as_deref()
            .is_some_and(|value| value != "unauthorized-origin" && url::Url::parse(value).is_err())
        || observation
            .target_href
            .as_deref()
            .is_some_and(|value| value != "unauthorized-origin" && url::Url::parse(value).is_err())
    {
        return Err(RenderError::new(
            "report_invalid",
            "step observation text or URL metadata is invalid",
        ));
    }
    if matches!(step.status, Outcome::Passed | Outcome::Failed) {
        let observed = observation.observed_url.as_deref().ok_or_else(|| {
            RenderError::new("report_invalid", "completed step omitted its observed URL")
        })?;
        let observed = url::Url::parse(observed).map_err(|_| {
            RenderError::new("report_invalid", "completed step observed an invalid URL")
        })?;
        if !origin.contains(&observed) {
            return Err(RenderError::new(
                "report_invalid",
                "passed or failed step observed an unauthorized origin",
            ));
        }
    }
    let declared_url = match action {
        ValidatedAction::Navigate { url }
        | ValidatedAction::CheckUrl { url }
        | ValidatedAction::FollowLink {
            expected_url: url, ..
        } => Some(safe_declared_url(url)),
        _ => None,
    };
    if declared_url.is_some() && observation.expected_url != declared_url {
        return Err(RenderError::new(
            "report_invalid",
            "step expected URL differs from the declared journey action",
        ));
    }
    match step.kind.as_str() {
        "navigate" if step.status == Outcome::Passed => {
            if observation.expected_url.is_none() || observation.observed_url.is_none() {
                return Err(RenderError::new(
                    "report_invalid",
                    "passed navigation observation is incomplete",
                ));
            }
        }
        "check_url" if matches!(step.status, Outcome::Passed | Outcome::Failed) => {
            if observation.expected_url.is_none()
                || observation.observed_url.is_none()
                || observation.matched != Some(step.status == Outcome::Passed)
            {
                return Err(RenderError::new(
                    "report_invalid",
                    "URL checkpoint observation contradicts its status",
                ));
            }
        }
        "check_text" if step.status == Outcome::Passed => {
            if observation.visible != Some(true)
                || observation.matched != Some(true)
                || observation.observed_text_sha256.is_none()
            {
                return Err(RenderError::new(
                    "report_invalid",
                    "passed text checkpoint observation is incomplete",
                ));
            }
        }
        "check_text" if step.status == Outcome::Failed => {
            if !matches!(
                (observation.visible, observation.matched),
                (Some(false), None) | (Some(true), Some(false))
            ) || (observation.visible == Some(true)
                && observation.observed_text_sha256.is_none())
            {
                return Err(RenderError::new(
                    "report_invalid",
                    "failed text checkpoint observation contradicts its status",
                ));
            }
        }
        "follow_link" if step.status == Outcome::Passed => {
            if observation.visible != Some(true)
                || observation.enabled != Some(true)
                || observation.matched != Some(true)
                || observation.action_state != Some(InputActionState::EffectVerified)
                || observation.action_grant_sha256.as_ref() != expected_action_binding.as_ref()
                || observation.action_command_sequence.is_none()
                || observation.before_url.is_none()
                || observation.target_href != observation.expected_url
                || observation.artifact_path.is_none()
            {
                return Err(RenderError::new(
                    "report_invalid",
                    "passed link action provenance is incomplete",
                ));
            }
        }
        "follow_link" if step.status == Outcome::Failed => {
            if observation.matched != Some(false)
                || !matches!(
                    (observation.visible, observation.enabled),
                    (Some(false), None) | (Some(true), Some(_))
                )
                || !matches!(
                    observation.action_state,
                    Some(InputActionState::NotAttempted | InputActionState::DriverAcknowledged)
                )
                || observation.action_grant_sha256.as_ref() != expected_action_binding.as_ref()
            {
                return Err(RenderError::new(
                    "report_invalid",
                    "failed link action provenance contradicts its status",
                ));
            }
        }
        _ => {}
    }
    if matches!(step.kind.as_str(), "capture" | "follow_link")
        && step.status == Outcome::Passed
        && (observation.visible != Some(true)
            || observation.artifact_path.is_none()
            || observation
                .capture_token
                .as_deref()
                .is_none_or(str::is_empty)
            || observation.box_command_sequence.is_none()
            || observation.screenshot_command_sequence
                != observation
                    .box_command_sequence
                    .map(|sequence| sequence.saturating_add(1)))
    {
        return Err(RenderError::new(
            "report_invalid",
            "passed capture provenance is incomplete",
        ));
    }
    Ok(())
}

fn safe_declared_url(url: &url::Url) -> String {
    let mut safe = url.clone();
    safe.set_query(None);
    safe.set_fragment(None);
    safe.to_string()
}

#[derive(Debug)]
struct VerifiedArtifacts {
    raw_by_step: HashMap<String, InputArtifact>,
    focused_by_step: HashMap<String, InputArtifact>,
    metadata_by_step: HashMap<String, InputArtifact>,
    trace: Option<InputArtifact>,
}

fn verify_artifacts(
    root: &Path,
    report: &InputRunReport,
    journey: &ValidatedJourney,
) -> Result<VerifiedArtifacts, RenderError> {
    let step_ids: HashSet<&str> = journey.steps.iter().map(|step| step.id.as_str()).collect();
    let evidence_ids: HashSet<&str> = journey
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.action,
                ValidatedAction::Capture { .. } | ValidatedAction::FollowLink { .. }
            )
        })
        .map(|step| step.id.as_str())
        .collect();
    let mut by_path = HashMap::new();
    let mut raw_by_step = HashMap::new();
    let mut focused_by_step = HashMap::new();
    let mut metadata_by_step = HashMap::new();
    let mut kind_step = HashSet::new();
    let mut metadata_bytes = HashMap::new();
    let mut png_inspections = HashMap::new();
    let mut total = 0u64;
    let mut trace = None;

    for artifact in &report.artifacts {
        validate_artifact_contract(artifact, &step_ids, &evidence_ids)?;
        total = total
            .checked_add(artifact.size_bytes)
            .ok_or_else(|| RenderError::new("artifact_invalid", "artifact sizes overflowed"))?;
        if total > MAX_ARTIFACT_TOTAL_BYTES {
            return Err(RenderError::new(
                "artifact_invalid",
                "run artifacts exceed the renderer verification limit",
            ));
        }
        if artifact.kind != "trace"
            && !kind_step.insert((artifact.kind.clone(), artifact.step_id.clone()))
        {
            return Err(RenderError::new(
                "report_invalid",
                "artifact kind and step bindings must be unique",
            ));
        }
        let path = contained_artifact(root, &artifact.path)?;
        let verified = verify_artifact_file(
            &path,
            artifact_maximum(&artifact.kind),
            matches!(
                artifact.kind.as_str(),
                "raw_screenshot" | "focused_screenshot" | "focus_metadata"
            ),
        )?;
        if verified.size_bytes != artifact.size_bytes {
            return Err(RenderError::new(
                "artifact_tampered",
                format!("artifact '{}' size or file type changed", artifact.path),
            ));
        }
        if verified.sha256 != artifact.sha256 {
            return Err(RenderError::new(
                "artifact_tampered",
                format!("artifact '{}' digest changed", artifact.path),
            ));
        }
        if let Some(bytes) = verified.bytes {
            if artifact.kind == "focus_metadata" {
                metadata_bytes.insert(artifact.path.clone(), bytes);
            } else {
                let inspection = focus::inspect_png(&bytes).map_err(|_| {
                    RenderError::new(
                        "artifact_tampered",
                        format!("artifact '{}' is not a supported PNG", artifact.path),
                    )
                })?;
                png_inspections.insert(artifact.path.clone(), inspection);
            }
        }
        if by_path
            .insert(artifact.path.clone(), artifact.clone())
            .is_some()
        {
            return Err(RenderError::new(
                "report_invalid",
                "artifact paths must be unique",
            ));
        }
        if artifact.kind == "raw_screenshot" {
            raw_by_step.insert(
                artifact.step_id.clone().expect("contract checked"),
                artifact.clone(),
            );
        }
        if artifact.kind == "focused_screenshot" {
            let step_id = artifact.step_id.clone().expect("contract checked");
            if focused_by_step.insert(step_id, artifact.clone()).is_some() {
                return Err(RenderError::new(
                    "report_invalid",
                    "a capture step has multiple focused screenshots",
                ));
            }
        }
        if artifact.kind == "focus_metadata" {
            metadata_by_step.insert(
                artifact.step_id.clone().expect("contract checked"),
                artifact.clone(),
            );
        }
        if artifact.kind == "trace" && trace.replace(artifact.clone()).is_some() {
            return Err(RenderError::new(
                "report_invalid",
                "run report contains multiple traces",
            ));
        }
    }

    for artifact in report
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind != "trace")
    {
        if artifact
            .source_artifact
            .as_ref()
            .is_some_and(|path| !by_path.contains_key(path))
        {
            return Err(RenderError::new(
                "report_invalid",
                "artifact source reference is dangling",
            ));
        }
    }

    for artifact in report
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "focus_metadata")
    {
        let bytes = metadata_bytes.get(&artifact.path).ok_or_else(|| {
            RenderError::new("artifact_missing", "focus metadata bytes were not verified")
        })?;
        let step = report
            .steps
            .iter()
            .find(|step| step.id == artifact.step_id.as_deref().unwrap_or(""))
            .ok_or_else(|| RenderError::new("report_invalid", "focus step is not executed"))?;
        verify_focus_metadata(artifact, bytes, &by_path, &png_inspections, journey, step)?;
    }
    for step in report.steps.iter().filter(|step| {
        matches!(step.kind.as_str(), "capture" | "follow_link")
            && step.observation.box_command_sequence.is_some()
    }) {
        let focused = focused_by_step.get(&step.id).ok_or_else(|| {
            RenderError::new(
                "artifact_missing",
                "focused evidence step has no focused screenshot",
            )
        })?;
        if step.observation.artifact_path.as_deref() != Some(&focused.path) {
            return Err(RenderError::new(
                "report_invalid",
                "focused evidence observation does not reference its screenshot",
            ));
        }
        let metadata_count = report
            .artifacts
            .iter()
            .filter(|artifact| {
                artifact.kind == "focus_metadata" && artifact.step_id.as_deref() == Some(&step.id)
            })
            .count();
        if metadata_count != 1 {
            return Err(RenderError::new(
                "artifact_missing",
                "focused evidence requires exactly one focus metadata artifact",
            ));
        }
    }
    for step in &report.steps {
        if let Some(path) = &step.observation.artifact_path
            && !by_path.contains_key(path)
        {
            return Err(RenderError::new(
                "report_invalid",
                "step observation artifact reference is dangling",
            ));
        }
    }
    if matches!(report.execution_outcome, Outcome::Passed | Outcome::Failed) && trace.is_none() {
        return Err(RenderError::new(
            "artifact_missing",
            "completed execution requires a verified trace",
        ));
    }
    Ok(VerifiedArtifacts {
        raw_by_step,
        focused_by_step,
        metadata_by_step,
        trace,
    })
}

fn validate_artifact_contract(
    artifact: &InputArtifact,
    step_ids: &HashSet<&str>,
    evidence_ids: &HashSet<&str>,
) -> Result<(), RenderError> {
    if artifact.size_bytes == 0
        || artifact.size_bytes > artifact_maximum(&artifact.kind)
        || !is_sha256(&artifact.sha256)
    {
        return Err(RenderError::new(
            "report_invalid",
            "artifact size or digest is invalid",
        ));
    }
    let valid = match artifact.kind.as_str() {
        "raw_screenshot" | "focused_screenshot" => artifact.media_type == "image/png",
        "focus_metadata" | "trace" => artifact.media_type == "application/json",
        _ => false,
    };
    if !valid {
        return Err(RenderError::new(
            "report_invalid",
            "artifact kind and media type are invalid",
        ));
    }
    if artifact.kind == "trace" {
        if artifact.step_id.is_some() || artifact.source_artifact.is_some() {
            return Err(RenderError::new(
                "report_invalid",
                "trace must not claim capture provenance",
            ));
        }
    } else {
        let Some(step_id) = artifact.step_id.as_deref() else {
            return Err(RenderError::new(
                "report_invalid",
                "capture artifact does not identify a capture step",
            ));
        };
        if !step_ids.contains(step_id) || !evidence_ids.contains(step_id) {
            return Err(RenderError::new(
                "report_invalid",
                "focused artifact references an unknown or non-evidence step",
            ));
        }
        if (artifact.kind == "raw_screenshot" && artifact.source_artifact.is_some())
            || (matches!(
                artifact.kind.as_str(),
                "focused_screenshot" | "focus_metadata"
            ) && artifact.source_artifact.is_none())
        {
            return Err(RenderError::new(
                "report_invalid",
                "capture artifact source provenance is invalid",
            ));
        }
    }
    Ok(())
}

fn artifact_maximum(kind: &str) -> u64 {
    match kind {
        "raw_screenshot" | "focused_screenshot" => MAX_SCREENSHOT_BYTES,
        "focus_metadata" => MAX_FOCUS_METADATA_BYTES,
        "trace" => MAX_TRACE_BYTES,
        _ => 0,
    }
}

fn verify_focus_metadata(
    artifact: &InputArtifact,
    bytes: &[u8],
    by_path: &HashMap<String, InputArtifact>,
    png_inspections: &HashMap<String, PngInspection>,
    journey: &ValidatedJourney,
    step: &InputStep,
) -> Result<(), RenderError> {
    let metadata: InputFocusMetadata = serde_json::from_slice(bytes).map_err(|error| {
        RenderError::new(
            "artifact_tampered",
            format!("focus metadata is invalid: {error}"),
        )
    })?;
    let step_id = artifact
        .step_id
        .as_deref()
        .ok_or_else(|| RenderError::new("report_invalid", "focus metadata has no capture step"))?;
    let declared = journey
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| RenderError::new("journey_drift", "focus step is no longer declared"))?;
    let alt_text = match &declared.action {
        ValidatedAction::Capture { alt_text, .. }
        | ValidatedAction::FollowLink { alt_text, .. } => alt_text,
        _ => {
            return Err(RenderError::new(
                "report_invalid",
                "focus metadata references a non-evidence step",
            ));
        }
    };
    let source = by_path.get(&metadata.source.path);
    let derivative = by_path.get(&metadata.derivative.path);
    let source_png = png_inspections.get(&metadata.source.path);
    let derivative_png = png_inspections.get(&metadata.derivative.path);
    let expected_width = source_png.map(|image| image.width).unwrap_or(0);
    let expected_height = source_png.map(|image| image.height).unwrap_or(0);
    let expected_scale_x = f64::from(expected_width) / metadata.viewport.width_css;
    let expected_scale_y = f64::from(expected_height) / metadata.viewport.height_css;
    let expected_target = mapped_rect(
        &metadata.target_box_css,
        expected_scale_x,
        expected_scale_y,
        expected_width,
        expected_height,
    );
    let padded = InputCssBox {
        x: metadata.target_box_css.x - 12.0,
        y: metadata.target_box_css.y - 12.0,
        width: metadata.target_box_css.width + 24.0,
        height: metadata.target_box_css.height + 24.0,
    };
    let expected_focus = mapped_rect(
        &padded,
        expected_scale_x,
        expected_scale_y,
        expected_width,
        expected_height,
    );
    let expected_outline_width =
        (3.0 * ((expected_scale_x + expected_scale_y) / 2.0)).ceil() as u32;
    if metadata.schema_version != 1
        || metadata.renderer_algorithm != "focus-overlay-v1"
        || metadata.status != "complete"
        || metadata.capture_step_id != step_id
        || metadata.alt_text != *alt_text
        || metadata.coordinate_space != "top_level_viewport"
        || metadata.output_color_type != "rgba8"
        || metadata.png_crate_version != "0.18.1"
        || metadata.png_compression != "fast"
        || metadata.png_filter != "paeth"
        || !matches!(
            metadata.decoded_color_type.as_str(),
            "rgba" | "rgb" | "grayscale" | "grayscalealpha"
        )
        || metadata.image_width_px == 0
        || metadata.image_height_px == 0
        || metadata.image_width_px != expected_width
        || metadata.image_height_px != expected_height
        || source_png.is_none_or(|image| image.color_type != metadata.decoded_color_type)
        || derivative_png.is_none_or(|image| {
            image.width != expected_width
                || image.height != expected_height
                || image.color_type != "rgba"
        })
        || !metadata.viewport.valid()
        || !metadata.target_box_css.valid()
        || expected_target.as_ref().is_none_or(|(rect, clipped)| {
            rect != &metadata.target_rect_px || clipped != &metadata.clipped_edges
        })
        || expected_focus.as_ref().map(|(rect, _)| rect) != Some(&metadata.focus_rect_px)
        || !metadata.scale_x.is_finite()
        || metadata.scale_x <= 0.0
        || !metadata.scale_y.is_finite()
        || metadata.scale_y <= 0.0
        || metadata.scale_x != expected_scale_x
        || metadata.scale_y != expected_scale_y
        || (metadata.scale_x - metadata.viewport.device_scale_factor).abs()
            > metadata.viewport.device_scale_factor * 0.01
        || (metadata.scale_y - metadata.viewport.device_scale_factor).abs()
            > metadata.viewport.device_scale_factor * 0.01
        || metadata.padding_css != 12.0
        || metadata.mask_rgba != [0, 0, 0, 166]
        || metadata.outline_rgba != [255, 45, 45, 255]
        || metadata.outline_width_css != 3.0
        || metadata.outline_width_px != expected_outline_width.max(2)
        || metadata.capture_token != step.observation.capture_token.as_deref().unwrap_or("")
        || Some(metadata.box_command_sequence) != step.observation.box_command_sequence
        || Some(metadata.screenshot_command_sequence)
            != step.observation.screenshot_command_sequence
        || step.observation.viewport.as_ref() != Some(&metadata.viewport)
        || step.observation.target_box_css.as_ref() != Some(&metadata.target_box_css)
        || metadata.screenshot_command_sequence != metadata.box_command_sequence.saturating_add(1)
        || source.is_none_or(|item| {
            item.kind != "raw_screenshot"
                || item.step_id.as_deref() != Some(step_id)
                || item.sha256 != metadata.source.sha256
                || item.size_bytes != metadata.source.size_bytes
                || item.media_type != metadata.source.media_type
        })
        || derivative.is_none_or(|item| {
            item.kind != "focused_screenshot"
                || item.step_id.as_deref() != Some(step_id)
                || item.source_artifact.as_deref() != Some(metadata.source.path.as_str())
                || item.sha256 != metadata.derivative.sha256
                || item.size_bytes != metadata.derivative.size_bytes
                || item.media_type != metadata.derivative.media_type
        })
        || artifact.source_artifact.as_deref() != Some(metadata.source.path.as_str())
    {
        return Err(RenderError::new(
            "artifact_tampered",
            "focus metadata no longer proves the declared focused image",
        ));
    }
    Ok(())
}

fn mapped_rect(
    area: &InputCssBox,
    scale_x: f64,
    scale_y: f64,
    width: u32,
    height: u32,
) -> Option<(InputPixelRect, InputClippedEdges)> {
    let raw_left = (area.x * scale_x).floor();
    let raw_top = (area.y * scale_y).floor();
    let raw_right = ((area.x + area.width) * scale_x).ceil();
    let raw_bottom = ((area.y + area.height) * scale_y).ceil();
    if [raw_left, raw_top, raw_right, raw_bottom]
        .iter()
        .any(|value| !value.is_finite())
    {
        return None;
    }
    let rect = InputPixelRect {
        left: raw_left.clamp(0.0, f64::from(width)) as u32,
        top: raw_top.clamp(0.0, f64::from(height)) as u32,
        right: raw_right.clamp(0.0, f64::from(width)) as u32,
        bottom: raw_bottom.clamp(0.0, f64::from(height)) as u32,
    };
    if !rect.valid(width, height) {
        return None;
    }
    Some((
        rect,
        InputClippedEdges {
            left: raw_left < 0.0,
            top: raw_top < 0.0,
            right: raw_right > f64::from(width),
            bottom: raw_bottom > f64::from(height),
        },
    ))
}

fn contained_artifact(root: &Path, relative: &str) -> Result<PathBuf, RenderError> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || relative.contains('\\')
        || relative.contains(':')
        || relative.chars().any(char::is_control)
        || relative.split('/').any(|part| part.is_empty())
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RenderError::new(
            "artifact_path_escape",
            "artifact path must be a normalized relative path",
        ));
    }
    let candidate = root.join(path);
    let mut current = root.to_path_buf();
    for component in path.components() {
        let Component::Normal(component) = component else {
            unreachable!("components were validated")
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| RenderError::new("artifact_missing", error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(RenderError::new(
                "artifact_path_escape",
                "artifact path must not traverse a symlink",
            ));
        }
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| RenderError::new("artifact_missing", error.to_string()))?;
    if !canonical.starts_with(root) {
        return Err(RenderError::new(
            "artifact_path_escape",
            "artifact resolves outside the run directory",
        ));
    }
    Ok(canonical)
}

struct VerifiedFile {
    size_bytes: u64,
    sha256: String,
    bytes: Option<Vec<u8>>,
}

fn verify_artifact_file(
    path: &Path,
    maximum: u64,
    retain_bytes: bool,
) -> Result<VerifiedFile, RenderError> {
    let mut file = fs::File::open(path)
        .map_err(|error| RenderError::new("artifact_invalid", error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| RenderError::new("artifact_invalid", error.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(RenderError::new(
            "artifact_tampered",
            format!("artifact must contain 1 to {maximum} bytes"),
        ));
    }
    let mut digest = Sha256::new();
    let mut retained = retain_bytes.then(|| Vec::with_capacity(metadata.len() as usize));
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| RenderError::new("artifact_invalid", error.to_string()))?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > maximum {
            return Err(RenderError::new(
                "artifact_tampered",
                "artifact changed while it was being verified",
            ));
        }
        digest.update(&buffer[..count]);
        if let Some(bytes) = &mut retained {
            bytes.extend_from_slice(&buffer[..count]);
        }
    }
    if size != metadata.len() {
        return Err(RenderError::new(
            "artifact_tampered",
            "artifact changed while it was being verified",
        ));
    }
    Ok(VerifiedFile {
        size_bytes: size,
        sha256: digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        bytes: retained,
    })
}

fn build_guide(
    root: &Path,
    report: InputRunReport,
    journey: &ValidatedJourney,
    artifacts: &VerifiedArtifacts,
    rendered_journey: RenderedJourney,
    report_sha256: String,
) -> Result<RenderReport, RenderError> {
    let mut guide_steps = Vec::new();
    let mut guide_images = Vec::new();
    for (declared, executed) in journey.steps.iter().zip(&report.steps) {
        let Some(instruction) = declared.guide_instruction.as_deref() else {
            continue;
        };
        let (alt_text, action_executed) = match &declared.action {
            ValidatedAction::Capture { alt_text, .. } => (alt_text, false),
            ValidatedAction::FollowLink { alt_text, .. } => {
                if executed.observation.action_state != Some(InputActionState::EffectVerified) {
                    return publish(
                        root,
                        not_publishable(
                            &report,
                            rendered_journey,
                            report_sha256,
                            "guide_step_unverified",
                            "a guide action was not executed and effect-verified",
                        ),
                        Vec::new(),
                    );
                }
                (alt_text, true)
            }
            _ => {
                return publish(
                    root,
                    not_publishable(
                        &report,
                        rendered_journey,
                        report_sha256,
                        "guide_step_unverified",
                        "guide instructions must belong to passed focused-evidence steps",
                    ),
                    Vec::new(),
                );
            }
        };
        if executed.status != Outcome::Passed {
            return publish(
                root,
                not_publishable(
                    &report,
                    rendered_journey,
                    report_sha256,
                    "guide_step_unverified",
                    "a declared guide step did not pass",
                ),
                Vec::new(),
            );
        }
        let focused = artifacts.focused_by_step.get(&declared.id).ok_or_else(|| {
            RenderError::new("artifact_missing", "guide step has no focused image")
        })?;
        let focused_path = contained_artifact(root, &focused.path)?;
        let verified = verify_artifact_file(&focused_path, MAX_SCREENSHOT_BYTES, true)?;
        if verified.size_bytes != focused.size_bytes || verified.sha256 != focused.sha256 {
            return Err(RenderError::new(
                "artifact_tampered",
                "focused image changed before the guide snapshot was built",
            ));
        }
        let local_image = format!("{:03}-focused.png", guide_steps.len() + 1);
        guide_images.push((
            local_image.clone(),
            verified
                .bytes
                .expect("guide image bytes were explicitly retained"),
        ));
        guide_steps.push(GuideStep {
            number: guide_steps.len() as u32 + 1,
            title: declared.title.clone(),
            instruction: instruction.to_owned(),
            alt_text: alt_text.clone(),
            image_path: local_image,
            image_sha256: focused.sha256.clone(),
            action_executed,
        });
    }
    if guide_steps.is_empty() {
        return publish(
            root,
            not_publishable(
                &report,
                rendered_journey,
                report_sha256,
                "guide_steps_missing",
                "passed run has no guide instruction backed by a focused capture",
            ),
            Vec::new(),
        );
    }
    let markdown = guide_markdown(journey, &report, &guide_steps);
    let mut outputs = vec![output_from_bytes(
        "guide",
        "render/guide.md",
        "text/markdown",
        markdown.as_bytes(),
    )];
    let mut files = vec![("guide.md", markdown.into_bytes())];
    for (name, bytes) in &guide_images {
        outputs.push(output_from_bytes(
            "guide_image",
            &format!("render/{name}"),
            "image/png",
            bytes,
        ));
    }
    for (name, bytes) in &guide_images {
        files.push((name.as_str(), bytes.clone()));
    }
    let result = RenderReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status: RenderStatus::GuideReady,
        publishable: true,
        run_id: Some(report.run_id),
        report_sha256: Some(report_sha256),
        journey: Some(rendered_journey),
        reason: RenderReason {
            code: "guide_rendered".to_owned(),
            message: "verified passed steps produced a deterministic guide".to_owned(),
        },
        guide_steps: guide_steps.len() as u32,
        findings: 0,
        outputs,
    };
    publish(root, result, files)
}

fn not_publishable(
    report: &InputRunReport,
    journey: RenderedJourney,
    report_sha256: String,
    code: &str,
    message: &str,
) -> RenderReport {
    RenderReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status: RenderStatus::NotPublishable,
        publishable: false,
        run_id: Some(report.run_id.clone()),
        report_sha256: Some(report_sha256),
        journey: Some(journey),
        reason: RenderReason {
            code: code.to_owned(),
            message: message.to_owned(),
        },
        guide_steps: 0,
        findings: 0,
        outputs: Vec::new(),
    }
}

#[derive(Debug)]
struct GuideStep {
    number: u32,
    title: String,
    instruction: String,
    alt_text: String,
    image_path: String,
    image_sha256: String,
    action_executed: bool,
}

fn guide_markdown(
    journey: &ValidatedJourney,
    report: &InputRunReport,
    steps: &[GuideStep],
) -> String {
    let verification_scope = if journey.schema_version >= 3 {
        "the declared checkpoints, authorized actions, and focused evidence"
    } else {
        "the declared checkpoints and captures"
    };
    let mut markdown = format!(
        "# {}\n\n{}\n\nDeclared expected outcome: {}\n\nCrawlson run `{}` passed {} for journey `{}` revision {} (`{}`). Free-form outcome prose is authored context, not an additional executed assertion.\n",
        escape_markdown(&journey.meta.title),
        escape_markdown(&journey.meta.purpose),
        escape_markdown(&journey.meta.expected_outcome),
        escape_code(&report.run_id),
        verification_scope,
        escape_code(&journey.meta.id),
        journey.meta.revision,
        journey.source_sha256
    );
    for step in steps {
        let verification = if step.action_executed {
            "Crawlson executed this highlighted link action once and verified its exact declared same-origin destination."
        } else {
            "The highlighted action area was observed in the read-only run. The authored instruction describes the reader's next action; Crawlson does not claim that action was executed."
        };
        markdown.push_str(&format!(
            "\n## {}. {}\n\n{}\n\n![{}]({})\n\n{}\n\nEvidence SHA-256: `{}`\n",
            step.number,
            escape_markdown(&step.title),
            escape_markdown(&step.instruction),
            escape_alt_text(&step.alt_text),
            markdown_path(&step.image_path),
            verification,
            step.image_sha256
        ));
    }
    markdown
}

fn build_findings(
    root: &Path,
    report: InputRunReport,
    journey: &ValidatedJourney,
    artifacts: &VerifiedArtifacts,
    rendered_journey: RenderedJourney,
    report_sha256: String,
) -> Result<RenderReport, RenderError> {
    if report.steps.iter().any(|step| {
        step.status != Outcome::Passed
            && !(step.status == Outcome::Failed
                && matches!(
                    step.kind.as_str(),
                    "check_url" | "check_text" | "follow_link"
                ))
    }) {
        let mut result = not_publishable(
            &report,
            rendered_journey,
            report_sha256,
            "run_incomplete",
            "failed or incomplete evidence steps cannot produce a finding",
        );
        result.status = RenderStatus::Error;
        return publish(root, result, Vec::new());
    }
    let failed_indices: Vec<usize> = report
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            step.status == Outcome::Failed
                && matches!(
                    step.kind.as_str(),
                    "check_url" | "check_text" | "follow_link"
                )
        })
        .map(|(index, _)| index)
        .collect();
    if failed_indices.is_empty() {
        return publish(
            root,
            not_publishable(
                &report,
                rendered_journey,
                report_sha256,
                "deterministic_finding_missing",
                "failed run has no deterministic checkpoint failure to publish",
            ),
            Vec::new(),
        );
    }

    let mut findings = Vec::new();
    for (finding_index, failed_index) in failed_indices.iter().copied().enumerate() {
        let failed = &report.steps[failed_index];
        let reproduction_steps = report.steps[..=failed_index]
            .iter()
            .zip(&journey.steps[..=failed_index])
            .map(|(step, declared)| ReproductionStep::new(step, declared))
            .collect();
        let mut evidence = vec![FindingEvidence {
            kind: "run_report".to_owned(),
            path: "report.json".to_owned(),
            sha256: report_sha256.clone(),
            capture_step_id: None,
            association_source: None,
        }];
        if let Some(trace) = &artifacts.trace {
            evidence.push(FindingEvidence::from_artifact(trace));
        }
        if failed.kind == "follow_link" {
            for artifact in [
                artifacts.raw_by_step.get(&failed.id),
                artifacts.focused_by_step.get(&failed.id),
                artifacts.metadata_by_step.get(&failed.id),
            ]
            .into_iter()
            .flatten()
            {
                evidence.push(FindingEvidence {
                    kind: artifact.kind.clone(),
                    path: artifact.path.clone(),
                    sha256: artifact.sha256.clone(),
                    capture_step_id: Some(failed.id.clone()),
                    association_source: Some("action.preflight"),
                });
            }
        }
        for (declared, executed) in journey.steps.iter().zip(&report.steps) {
            if declared
                .evidence_for
                .iter()
                .any(|step_id| step_id == &failed.id)
                && executed.status == Outcome::Passed
                && let Some(focused) = artifacts.focused_by_step.get(&declared.id)
            {
                for artifact in [
                    artifacts.raw_by_step.get(&declared.id),
                    Some(focused),
                    artifacts.metadata_by_step.get(&declared.id),
                ]
                .into_iter()
                .flatten()
                {
                    evidence.push(FindingEvidence {
                        kind: artifact.kind.clone(),
                        path: artifact.path.clone(),
                        sha256: artifact.sha256.clone(),
                        capture_step_id: Some(declared.id.clone()),
                        association_source: Some("journey.evidence_for"),
                    });
                }
            }
        }
        findings.push(Finding {
            id: format!("finding-{:03}-{}", finding_index + 1, failed.id),
            severity: "untriaged",
            severity_source: "not_assessed",
            kind: finding_kind(failed),
            symptom: failure_symptom(failed),
            step: FindingStep {
                sequence: failed.sequence,
                id: failed.id.clone(),
                title: failed.title.clone(),
            },
            checkpoint: checkpoint(&journey.steps[failed_index].action, failed),
            reproduction_steps,
            evidence,
        });
    }
    let document = FindingsDocument {
        schema_version: if journey.schema_version >= 3 { 2 } else { 1 },
        run_id: report.run_id.clone(),
        journey: rendered_journey.clone(),
        findings,
    };
    let mut json = serde_json::to_vec_pretty(&document)
        .map_err(|error| RenderError::new("render_write_failed", error.to_string()))?;
    json.push(b'\n');
    let markdown = findings_markdown(journey, &document);
    let outputs = vec![
        output_from_bytes(
            "findings_json",
            "render/findings.json",
            "application/json",
            &json,
        ),
        output_from_bytes(
            "findings_markdown",
            "render/findings.md",
            "text/markdown",
            markdown.as_bytes(),
        ),
    ];
    let result = RenderReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status: RenderStatus::FindingsReady,
        publishable: true,
        run_id: Some(report.run_id),
        report_sha256: Some(report_sha256),
        journey: Some(rendered_journey),
        reason: RenderReason {
            code: "findings_rendered".to_owned(),
            message: "deterministic checkpoint failures produced evidence-backed findings"
                .to_owned(),
        },
        guide_steps: 0,
        findings: document.findings.len() as u32,
        outputs,
    };
    publish(
        root,
        result,
        vec![
            ("findings.json", json),
            ("findings.md", markdown.into_bytes()),
        ],
    )
}

#[derive(Debug, Clone, Serialize)]
struct FindingsDocument {
    schema_version: u8,
    run_id: String,
    journey: RenderedJourney,
    findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize)]
struct Finding {
    id: String,
    severity: &'static str,
    severity_source: &'static str,
    kind: &'static str,
    symptom: &'static str,
    step: FindingStep,
    checkpoint: Checkpoint,
    reproduction_steps: Vec<ReproductionStep>,
    evidence: Vec<FindingEvidence>,
}

#[derive(Debug, Clone, Serialize)]
struct FindingStep {
    sequence: u32,
    id: String,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
struct Checkpoint {
    expected: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    comparison: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_text_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ReproductionStep {
    sequence: u32,
    id: String,
    title: String,
    kind: String,
    status: Outcome,
    action: ReproductionAction,
}

impl ReproductionStep {
    fn new(step: &InputStep, declared: &crate::journey::ValidatedStep) -> Self {
        Self {
            sequence: step.sequence,
            id: step.id.clone(),
            title: step.title.clone(),
            kind: step.kind.clone(),
            status: step.status,
            action: ReproductionAction::from(&declared.action),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReproductionAction {
    Navigate {
        path: String,
    },
    CheckUrl {
        path: String,
    },
    CheckText {
        selector: String,
        expected: String,
        comparison: &'static str,
    },
    Capture {
        selector: String,
    },
    FollowLink {
        selector: String,
        expected_path: String,
    },
}

impl From<&ValidatedAction> for ReproductionAction {
    fn from(action: &ValidatedAction) -> Self {
        match action {
            ValidatedAction::Navigate { url } => Self::Navigate {
                path: url.path().to_owned(),
            },
            ValidatedAction::CheckUrl { url } => Self::CheckUrl {
                path: url.path().to_owned(),
            },
            ValidatedAction::CheckText {
                selector,
                expected,
                comparison,
            } => Self::CheckText {
                selector: selector.clone(),
                expected: expected.clone(),
                comparison: match comparison {
                    crate::journey::TextComparison::Exact => "exact",
                    crate::journey::TextComparison::Contains => "contains",
                },
            },
            ValidatedAction::Capture { selector, .. } => Self::Capture {
                selector: selector.clone(),
            },
            ValidatedAction::FollowLink {
                selector,
                expected_url,
                ..
            } => Self::FollowLink {
                selector: selector.clone(),
                expected_path: expected_url.path().to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct FindingEvidence {
    kind: String,
    path: String,
    sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    capture_step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    association_source: Option<&'static str>,
}

impl FindingEvidence {
    fn from_artifact(artifact: &InputArtifact) -> Self {
        Self {
            kind: artifact.kind.clone(),
            path: artifact.path.clone(),
            sha256: artifact.sha256.clone(),
            capture_step_id: None,
            association_source: None,
        }
    }
}

fn reproduction_markdown(action: &ReproductionAction, failed_finding_kind: Option<&str>) -> String {
    match action {
        ReproductionAction::Navigate { path } => {
            format!("Open path {}.", escape_markdown(path))
        }
        ReproductionAction::CheckUrl { path } => {
            format!("Confirm the current path is {}.", escape_markdown(path))
        }
        ReproductionAction::CheckText {
            selector,
            expected,
            comparison,
        } => format!(
            "Confirm `{}` is visible and its text {} {}.",
            escape_code(selector),
            if *comparison == "exact" {
                "exactly matches"
            } else {
                "contains"
            },
            escape_markdown(expected)
        ),
        ReproductionAction::Capture { selector } => {
            format!(
                "Observe the highlighted action area at `{}`.",
                escape_code(selector)
            )
        }
        ReproductionAction::FollowLink {
            selector,
            expected_path,
        } => match failed_finding_kind {
            Some("link_not_visible") => format!(
                "Inspect `{}`; the declared link should be visible.",
                escape_code(selector)
            ),
            Some("link_not_enabled") => format!(
                "Inspect `{}`; the declared link should be enabled.",
                escape_code(selector)
            ),
            Some("link_target_invalid") => format!(
                "Inspect the href on `{}`; it should be a valid bounded URL.",
                escape_code(selector)
            ),
            Some("link_destination_mismatch") => format!(
                "Inspect the href on `{}` and compare it with {}.",
                escape_code(selector),
                escape_markdown(expected_path)
            ),
            _ => format!(
                "Select the link at `{}` and confirm it reaches {}.",
                escape_code(selector),
                escape_markdown(expected_path)
            ),
        },
    }
}

fn failure_symptom(step: &InputStep) -> &'static str {
    match step.kind.as_str() {
        "check_url" => "Observed URL did not match the declared checkpoint.",
        "check_text" if step.observation.visible == Some(false) => {
            "Declared visible text was not visible."
        }
        "check_text" => "Visible text did not match the declared checkpoint.",
        "follow_link" if step.observation.visible == Some(false) => {
            "The declared link was not visible."
        }
        "follow_link" if step.observation.enabled == Some(false) => {
            "The declared link was visible but not enabled."
        }
        "follow_link"
            if step.observation.action_state == Some(InputActionState::DriverAcknowledged) =>
        {
            "The link action completed, but the browser reached a different same-origin destination."
        }
        "follow_link" if step.observation.target_href.is_none() => {
            "The declared link did not expose a valid bounded destination."
        }
        "follow_link" => "The declared link href did not match its expected destination.",
        _ => "Declared checkpoint failed.",
    }
}

fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Passed => "passed",
        Outcome::Failed => "failed",
        Outcome::Blocked => "blocked",
        Outcome::Error => "error",
    }
}

fn finding_kind(step: &InputStep) -> &'static str {
    match step.kind.as_str() {
        "check_url" => "url_mismatch",
        "check_text" if step.observation.visible == Some(false) => "target_not_visible",
        "check_text" => "text_mismatch",
        "follow_link" if step.observation.visible == Some(false) => "link_not_visible",
        "follow_link" if step.observation.enabled == Some(false) => "link_not_enabled",
        "follow_link"
            if step.observation.action_state == Some(InputActionState::DriverAcknowledged) =>
        {
            "link_postcondition_mismatch"
        }
        "follow_link" if step.observation.target_href.is_none() => "link_target_invalid",
        "follow_link" => "link_destination_mismatch",
        _ => "checkpoint_failure",
    }
}

fn checkpoint(action: &ValidatedAction, step: &InputStep) -> Checkpoint {
    match action {
        ValidatedAction::CheckUrl { url } => Checkpoint {
            expected: url.path().to_owned(),
            comparison: Some("exact"),
            observed_path: step
                .observation
                .observed_url
                .as_deref()
                .map(safe_observed_path),
            visible: None,
            enabled: None,
            matched: step.observation.matched,
            action_state: None,
            observed_text_sha256: None,
        },
        ValidatedAction::CheckText {
            expected,
            comparison,
            ..
        } => Checkpoint {
            expected: expected.clone(),
            comparison: Some(match comparison {
                crate::journey::TextComparison::Exact => "exact",
                crate::journey::TextComparison::Contains => "contains",
            }),
            observed_path: None,
            visible: step.observation.visible,
            enabled: None,
            matched: step.observation.matched,
            action_state: None,
            observed_text_sha256: step.observation.observed_text_sha256.clone(),
        },
        ValidatedAction::FollowLink { expected_url, .. } => Checkpoint {
            expected: expected_url.path().to_owned(),
            comparison: Some("exact"),
            observed_path: match step.observation.action_state {
                Some(InputActionState::DriverAcknowledged) => {
                    step.observation.observed_url.as_deref()
                }
                Some(InputActionState::NotAttempted) => step.observation.target_href.as_deref(),
                _ => None,
            }
            .map(safe_observed_path),
            visible: step.observation.visible,
            enabled: step.observation.enabled,
            matched: step.observation.matched,
            action_state: step.observation.action_state.map(action_state_name),
            observed_text_sha256: None,
        },
        _ => unreachable!("findings are created only from checkpoint actions"),
    }
}

fn safe_observed_path(value: &str) -> String {
    if value == "unauthorized-origin" {
        return value.to_owned();
    }
    url::Url::parse(value)
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|_| "unavailable".to_owned())
}

fn findings_markdown(journey: &ValidatedJourney, document: &FindingsDocument) -> String {
    let mut markdown = format!(
        "# Findings: {}\n\nRun `{}` produced {} deterministic finding(s).\n",
        escape_markdown(&journey.meta.title),
        escape_code(&document.run_id),
        document.findings.len()
    );
    for finding in &document.findings {
        markdown.push_str(&format!(
            "\n## {}: {}\n\nSeverity: **{}** (not assessed)  \nKind: `{}`  \nExpected: {}  \nObserved: {}\n\n{}\n\n### Reproduce\n",
            escape_markdown(&finding.id),
            escape_markdown(&finding.step.title),
            finding.severity,
            finding.kind,
            escape_markdown(&finding.checkpoint.expected),
            checkpoint_observed_markdown(&finding.checkpoint),
            finding.symptom
        ));
        for step in &finding.reproduction_steps {
            markdown.push_str(&format!(
                "\n{}. **{}** — {} (`{}`): {}\n",
                step.sequence,
                outcome_name(step.status),
                escape_markdown(&step.title),
                escape_code(&step.id),
                reproduction_markdown(
                    &step.action,
                    (step.sequence == finding.step.sequence).then_some(finding.kind),
                )
            ));
        }
        markdown.push_str("\n### Evidence\n");
        for evidence in &finding.evidence {
            markdown.push_str(&format!(
                "\n- [{}](../{}) — `{}`\n",
                escape_markdown(&evidence.kind),
                markdown_path(&evidence.path),
                evidence.sha256
            ));
        }
    }
    markdown
}

fn checkpoint_observed_markdown(checkpoint: &Checkpoint) -> String {
    let mut values = Vec::new();
    if let Some(path) = &checkpoint.observed_path {
        values.push(format!("path {}", escape_markdown(path)));
    }
    if let Some(visible) = checkpoint.visible {
        values.push(format!("visible `{visible}`"));
    }
    if let Some(enabled) = checkpoint.enabled {
        values.push(format!("enabled `{enabled}`"));
    }
    if let Some(matched) = checkpoint.matched {
        values.push(format!("matched `{matched}`"));
    }
    if let Some(digest) = &checkpoint.observed_text_sha256 {
        values.push(format!("text SHA-256 `{digest}`"));
    }
    if values.is_empty() {
        "no value recorded".to_owned()
    } else {
        values.join(", ")
    }
}

fn action_state_name(state: InputActionState) -> &'static str {
    match state {
        InputActionState::NotAttempted => "not_attempted",
        InputActionState::DriverAcknowledged => "driver_acknowledged",
        InputActionState::EffectVerified => "effect_verified",
        InputActionState::EffectUnknown => "effect_unknown",
    }
}

fn publish(
    root: &Path,
    report: RenderReport,
    mut files: Vec<(&str, Vec<u8>)>,
) -> Result<RenderReport, RenderError> {
    let destination = root.join("render");
    let mut report_bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| RenderError::new("render_write_failed", error.to_string()))?;
    report_bytes.push(b'\n');
    files.push(("render-report.json", report_bytes));

    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            if existing_render_matches(&destination, &files)? {
                return Ok(report);
            }
            return Err(RenderError::new(
                "render_output_conflict",
                "existing render output differs; prior renders are never overwritten",
            ));
        }
        Ok(_) => {
            return Err(RenderError::new(
                "render_output_conflict",
                "render output path is not a real directory",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RenderError::new("render_write_failed", error.to_string()));
        }
    }
    let staging = tempfile::Builder::new()
        .prefix(".crawlson-render-")
        .tempdir_in(root)
        .map_err(|error| RenderError::new("render_write_failed", error.to_string()))?;
    for (name, bytes) in &files {
        atomic_write(&staging.path().join(name), bytes)?;
    }
    let staging_path = staging.keep();
    if let Err(error) = fs::rename(&staging_path, &destination) {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(RenderError::new("render_write_failed", error.to_string()));
    }
    Ok(report)
}

fn existing_render_matches(
    directory: &Path,
    expected: &[(&str, Vec<u8>)],
) -> Result<bool, RenderError> {
    let entries = fs::read_dir(directory)
        .map_err(|error| RenderError::new("render_output_conflict", error.to_string()))?;
    let mut seen = HashSet::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| RenderError::new("render_output_conflict", error.to_string()))?;
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            return Ok(false);
        };
        let Some((_, bytes)) = expected.iter().find(|(expected, _)| *expected == name) else {
            return Ok(false);
        };
        let actual =
            read_regular_bounded(&entry.path(), bytes.len() as u64, "existing render output")?;
        if actual != *bytes || !seen.insert(name) {
            return Ok(false);
        }
    }
    Ok(seen.len() == expected.len())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), RenderError> {
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| RenderError::new("render_write_failed", error.to_string()))?;
    file.write_all(bytes)
        .map_err(|error| RenderError::new("render_write_failed", error.to_string()))?;
    file.commit()
        .map_err(|error| RenderError::new("render_write_failed", error.to_string()))
}

fn output_from_bytes(
    kind: &'static str,
    path: &str,
    media_type: &'static str,
    bytes: &[u8],
) -> RenderedOutput {
    RenderedOutput {
        kind,
        path: path.to_owned(),
        size_bytes: bytes.len() as u64,
        media_type,
        sha256: journey::hex_digest(bytes),
    }
}

fn action_kind(action: &ValidatedAction) -> &'static str {
    match action {
        ValidatedAction::Navigate { .. } => "navigate",
        ValidatedAction::CheckUrl { .. } => "check_url",
        ValidatedAction::CheckText { .. } => "check_text",
        ValidatedAction::FollowLink { .. } => "follow_link",
        ValidatedAction::Capture { .. } => "capture",
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn escape_markdown(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if "\\`*_{}[]<>()#+-.!|".contains(character) {
                vec!['\\', character]
            } else if character.is_control() || is_bidi_control(character) {
                vec![' ']
            } else {
                vec![character]
            }
        })
        .collect()
}

fn escape_alt_text(value: &str) -> String {
    normalize_text(value)
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn escape_code(value: &str) -> String {
    normalize_text(value).replace('`', "'")
}

fn normalize_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || is_bidi_control(character) {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn markdown_path(value: &str) -> String {
    value
        .split('/')
        .map(percent_encode_path_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            output.push(char::from(byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRunReport {
    schema_version: u8,
    crawlson_version: String,
    run_id: String,
    run_directory: String,
    journey: InputJourney,
    target_origin: Option<String>,
    action_authorization: Option<InputActionAuthorization>,
    authentication: Option<InputAuthentication>,
    started_at_unix_ms: u64,
    finished_at_unix_ms: u64,
    duration_ms: u64,
    outcome: Outcome,
    execution_outcome: Outcome,
    reason: InputReason,
    execution_reason: InputReason,
    driver: InputDriver,
    steps: Vec<InputStep>,
    artifacts: Vec<InputArtifact>,
    diagnostics: Option<InputDiagnostics>,
    cleanup: InputCleanup,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputAuthentication {
    provider: String,
    role: String,
    verification_step: String,
    status: InputAuthenticationStatus,
    binding_sha256: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InputAuthenticationStatus {
    Missing,
    Unsupported,
    Invalid,
    LoadFailed,
    Blocked,
    Verified,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputActionAuthorization {
    required: Vec<String>,
    granted: Vec<String>,
    binding_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputJourney {
    source_path: String,
    source_sha256: Option<String>,
    id: Option<String>,
    revision: Option<u32>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InputReason {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDriver {
    name: String,
    version: Option<String>,
    session: Option<String>,
    commands: Vec<InputDriverCommand>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDriverCommand {
    sequence: u32,
    capability: String,
    duration_ms: u64,
    exit_code: Option<i32>,
    upstream_success: bool,
    stdout_bytes: u64,
    stdout_captured_bytes: u64,
    stdout_captured_sha256: String,
    stderr_bytes: u64,
    stderr_captured_bytes: u64,
    stderr_captured_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputStep {
    sequence: u32,
    id: String,
    title: String,
    kind: String,
    status: Outcome,
    started_at_unix_ms: u64,
    duration_ms: u64,
    observation: InputObservation,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputObservation {
    expected_url: Option<String>,
    observed_url: Option<String>,
    matched: Option<bool>,
    visible: Option<bool>,
    enabled: Option<bool>,
    observed_text_sha256: Option<String>,
    artifact_path: Option<String>,
    target_box_css: Option<InputCssBox>,
    viewport: Option<InputViewport>,
    capture_token: Option<String>,
    box_command_sequence: Option<u32>,
    screenshot_command_sequence: Option<u32>,
    detail: Option<String>,
    action_state: Option<InputActionState>,
    action_grant_sha256: Option<String>,
    action_command_sequence: Option<u32>,
    before_url: Option<String>,
    target_href: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputArtifact {
    kind: String,
    path: String,
    size_bytes: u64,
    media_type: String,
    sha256: String,
    step_id: Option<String>,
    source_artifact: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDiagnostics {
    console_messages: u64,
    console_sha256: String,
    page_errors: u64,
    page_errors_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputCleanup {
    attempted: bool,
    status: CleanupStatus,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Passed,
    Failed,
    Blocked,
    Error,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InputActionState {
    NotAttempted,
    DriverAcknowledged,
    EffectVerified,
    EffectUnknown,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CleanupStatus {
    NotNeeded,
    Passed,
    Failed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFocusMetadata {
    schema_version: u8,
    renderer_algorithm: String,
    status: String,
    capture_step_id: String,
    capture_token: String,
    box_command_sequence: u32,
    screenshot_command_sequence: u32,
    alt_text: String,
    coordinate_space: String,
    source: InputImageArtifact,
    derivative: InputImageArtifact,
    decoded_color_type: String,
    output_color_type: String,
    png_crate_version: String,
    png_compression: String,
    png_filter: String,
    image_width_px: u32,
    image_height_px: u32,
    viewport: InputViewport,
    scale_x: f64,
    scale_y: f64,
    target_box_css: InputCssBox,
    target_rect_px: InputPixelRect,
    focus_rect_px: InputPixelRect,
    clipped_edges: InputClippedEdges,
    padding_css: f64,
    mask_rgba: [u8; 4],
    outline_rgba: [u8; 4],
    outline_width_css: f64,
    outline_width_px: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputImageArtifact {
    path: String,
    size_bytes: u64,
    media_type: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct InputCssBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl InputCssBox {
    fn valid(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct InputViewport {
    width_css: f64,
    height_css: f64,
    device_scale_factor: f64,
    scroll_x_css: Option<f64>,
    scroll_y_css: Option<f64>,
}

impl InputViewport {
    fn valid(&self) -> bool {
        self.width_css.is_finite()
            && self.width_css > 0.0
            && self.height_css.is_finite()
            && self.height_css > 0.0
            && self.device_scale_factor.is_finite()
            && self.device_scale_factor > 0.0
            && self.scroll_x_css.is_none_or(f64::is_finite)
            && self.scroll_y_css.is_none_or(f64::is_finite)
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InputPixelRect {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl InputPixelRect {
    fn valid(&self, width: u32, height: u32) -> bool {
        self.left < self.right
            && self.top < self.bottom
            && self.right <= width
            && self.bottom <= height
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InputClippedEdges {
    left: bool,
    top: bool,
    right: bool,
    bottom: bool,
}
