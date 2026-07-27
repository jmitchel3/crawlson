use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::doctor::{self, CheckStatus, DoctorOptions};
use crate::driver::{
    AgentBrowserDriver, BrowserDriver, DiagnosticsSummary, DriverCommandRecord, DriverError,
};
use crate::focus::{self, CssBox, FocusRequest, Viewport};
use crate::journey::{self, Origin, TextComparison, ValidatedAction, ValidatedJourney, hex_digest};
use crate::{CommandResult, VERSION};

pub const EXIT_PASSED: u8 = 0;
pub const EXIT_FAILED: u8 = 1;
pub const EXIT_BLOCKED: u8 = 3;
pub const EXIT_ERROR: u8 = 4;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub journey_path: PathBuf,
    pub allowed_origin: Option<String>,
    pub output_directory: PathBuf,
    pub agent_browser: Option<PathBuf>,
    pub action_timeout: Duration,
    pub run_timeout: Duration,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Passed,
    Failed,
    Blocked,
    Error,
}

impl RunOutcome {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Passed => EXIT_PASSED,
            Self::Failed => EXIT_FAILED,
            Self::Blocked => EXIT_BLOCKED,
            Self::Error => EXIT_ERROR,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub schema_version: u8,
    pub crawlson_version: &'static str,
    pub run_id: String,
    pub run_directory: String,
    pub journey: JourneyProvenance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_origin: Option<String>,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: u64,
    pub duration_ms: u64,
    pub outcome: RunOutcome,
    pub execution_outcome: RunOutcome,
    pub reason: Reason,
    pub execution_reason: Reason,
    pub driver: DriverReport,
    pub steps: Vec<StepReport>,
    pub artifacts: Vec<ArtifactRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<DiagnosticsSummary>,
    pub cleanup: CleanupReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct JourneyProvenance {
    pub source_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverReport {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub commands: Vec<DriverCommandRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub sequence: u32,
    pub id: String,
    pub title: String,
    pub kind: &'static str,
    pub status: RunOutcome,
    pub started_at_unix_ms: u64,
    pub duration_ms: u64,
    pub observation: StepObservation,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StepObservation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_text_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_box_css: Option<CssBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_command_sequence: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_command_sequence: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactRecord {
    pub kind: &'static str,
    pub path: String,
    pub size_bytes: u64,
    pub media_type: &'static str,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_artifact: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupReport {
    pub attempted: bool,
    pub status: CleanupStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStatus {
    NotNeeded,
    Passed,
    Failed,
}

impl RunReport {
    pub fn render(&self, json: bool) -> CommandResult {
        if json {
            let mut stdout = serde_json::to_string(self).expect("run report is serializable");
            stdout.push('\n');
            CommandResult {
                exit_code: self.outcome.exit_code(),
                stdout,
                stderr: String::new(),
            }
        } else {
            let mut stdout = format!(
                "Crawlson run {}: {}\nReason: {}\nArtifacts: {}\n",
                self.run_id,
                self.outcome.as_str(),
                self.reason.message,
                self.run_directory
            );
            for step in &self.steps {
                stdout.push_str(&format!(
                    "[{}] {}: {}\n",
                    step.status.as_str(),
                    step.id,
                    step.title
                ));
            }
            CommandResult {
                exit_code: self.outcome.exit_code(),
                stdout,
                stderr: String::new(),
            }
        }
    }
}

pub fn run(options: RunOptions) -> RunReport {
    let overall_start = Instant::now();
    let started_at = unix_ms();
    let source_path = safe_source_label(&options.journey_path);
    let (run_root, run_id) = match create_run_directory(&options.output_directory) {
        Ok(value) => value,
        Err(message) => {
            return empty_report(
                source_path,
                started_at,
                overall_start,
                "run_directory_unavailable",
                message,
            );
        }
    };
    let mut report = base_report(&run_root, &run_id, source_path, started_at);

    let loaded = match journey::load(&options.journey_path) {
        Ok(loaded) => loaded,
        Err(error) => {
            finish_error(&mut report, "journey_invalid", error.to_string());
            return finalize(report, overall_start, &run_root);
        }
    };
    report.journey.source_sha256 = Some(loaded.source_sha256.clone());
    let journey = match journey::validate(loaded) {
        Ok(journey) => journey,
        Err(error) => {
            finish_error(&mut report, "journey_invalid", error.to_string());
            return finalize(report, overall_start, &run_root);
        }
    };
    report.journey.id = Some(journey.meta.id.clone());
    report.journey.revision = Some(journey.meta.revision);
    report.target_origin = Some(journey.origin.to_string());

    let Some(allowed_origin) = options.allowed_origin.as_deref() else {
        finish_blocked(
            &mut report,
            "target_authorization_missing",
            format!(
                "explicit authorization is required; rerun with --allow-origin {}",
                journey.origin
            ),
        );
        return finalize(report, overall_start, &run_root);
    };
    match journey::parse_authorized_origin(allowed_origin) {
        Ok(allowed) if allowed == journey.origin => {}
        Ok(allowed) => {
            finish_blocked(
                &mut report,
                "target_not_authorized",
                format!(
                    "authorized origin {allowed} does not match journey origin {}",
                    journey.origin
                ),
            );
            return finalize(report, overall_start, &run_root);
        }
        Err(error) => {
            finish_blocked(
                &mut report,
                "target_authorization_invalid",
                error.to_string(),
            );
            return finalize(report, overall_start, &run_root);
        }
    }
    if journey.authentication.is_some() {
        finish_blocked(
            &mut report,
            "authentication_unavailable",
            "this journey requires authentication, which is not available in read-only journey v1"
                .to_owned(),
        );
        return finalize(report, overall_start, &run_root);
    }

    let doctor = doctor::run(DoctorOptions {
        executable: options.agent_browser,
    });
    let Some(check) = doctor.checks.first() else {
        finish_error(
            &mut report,
            "driver_unavailable",
            "agent-browser check did not return a result".to_owned(),
        );
        return finalize(report, overall_start, &run_root);
    };
    if check.status != CheckStatus::Pass {
        finish_error(
            &mut report,
            "driver_unavailable",
            "a supported agent-browser 0.26.x executable is required".to_owned(),
        );
        return finalize(report, overall_start, &run_root);
    }
    let Some(executable) = check.executable.as_deref() else {
        finish_error(
            &mut report,
            "driver_unavailable",
            "agent-browser executable path was not reported".to_owned(),
        );
        return finalize(report, overall_start, &run_root);
    };
    report.driver.version = check.detected_version.as_ref().map(ToString::to_string);
    let session = session_name(&run_id);
    report.driver.session = Some(session.clone());
    let mut driver = match AgentBrowserDriver::new(
        executable,
        &run_root,
        journey.origin.clone(),
        session,
        options.action_timeout,
        options.run_timeout,
    ) {
        Ok(driver) => driver,
        Err(error) => {
            finish_error(
                &mut report,
                "driver_unavailable",
                safe_driver_message(&error),
            );
            return finalize(report, overall_start, &run_root);
        }
    };

    execute(&journey, &run_root, &mut driver, &mut report);
    report.driver.commands = driver.records();
    finalize(report, overall_start, &run_root)
}

fn execute(
    journey: &ValidatedJourney,
    run_root: &Path,
    driver: &mut dyn BrowserDriver,
    report: &mut RunReport,
) {
    // Once the adapter exists, any command can start the session daemon before
    // its foreground client fails or times out. Always attempt owned-session
    // cleanup, including when prepare itself does not return success.
    let opened = true;
    let mut prepared = false;
    let mut trace_started = false;
    let mut primary = RunOutcome::Passed;
    let mut primary_reason = Reason {
        code: "journey_passed".to_owned(),
        message: "all read-only steps and required evidence completed".to_owned(),
    };

    match driver.prepare() {
        Ok(()) => prepared = true,
        Err(error) => set_driver_error(&mut primary, &mut primary_reason, error),
    }
    if primary == RunOutcome::Passed {
        match driver.start_trace() {
            Ok(()) => trace_started = true,
            Err(error) => set_driver_error(&mut primary, &mut primary_reason, error),
        }
    }

    let mut has_navigated = false;
    if primary == RunOutcome::Passed {
        for (index, step) in journey.steps.iter().enumerate() {
            let started = unix_ms();
            let timer = Instant::now();
            let mut observation = StepObservation::default();
            let kind = action_kind(&step.action);
            let result = execute_step(
                journey,
                run_root,
                driver,
                &step.id,
                index,
                &step.action,
                has_navigated,
                &mut observation,
                &mut report.artifacts,
            );
            if matches!(step.action, ValidatedAction::Navigate { .. }) && result.is_ok() {
                has_navigated = true;
            }
            let mut stop = false;
            let status = match result {
                Ok(StepResult::Passed) => RunOutcome::Passed,
                Ok(StepResult::Failed(message)) => {
                    if primary == RunOutcome::Passed {
                        primary = RunOutcome::Failed;
                        primary_reason = Reason {
                            code: "checkpoint_failed".to_owned(),
                            message,
                        };
                    }
                    RunOutcome::Failed
                }
                Ok(StepResult::Blocked(message)) => {
                    primary = RunOutcome::Blocked;
                    primary_reason = Reason {
                        code: "origin_not_authorized".to_owned(),
                        message,
                    };
                    stop = true;
                    RunOutcome::Blocked
                }
                Err(error) => {
                    set_driver_error(&mut primary, &mut primary_reason, error);
                    stop = true;
                    primary
                }
            };
            report.steps.push(StepReport {
                sequence: index as u32 + 1,
                id: step.id.clone(),
                title: step.title.clone(),
                kind,
                status,
                started_at_unix_ms: started,
                duration_ms: elapsed_ms(timer),
                observation,
            });
            if stop {
                break;
            }
        }
    }
    report.execution_outcome = primary;
    report.execution_reason = primary_reason.clone();
    report.outcome = primary;
    report.reason = primary_reason;

    if opened && prepared && journey.evidence.diagnostics && primary != RunOutcome::Blocked {
        match driver.diagnostics() {
            Ok(diagnostics) => report.diagnostics = Some(diagnostics),
            Err(error) => override_with_evidence_error(report, "diagnostics_failed", error),
        }
    }
    if trace_started {
        let path = run_root.join("evidence").join("trace.json");
        match driver.stop_trace(&path) {
            Ok(path) => {
                match artifact_record(run_root, &path, "trace", "application/json", None, None) {
                    Ok(artifact) => report.artifacts.push(artifact),
                    Err(message) => {
                        override_with_message(report, "trace_artifact_invalid", message.to_string())
                    }
                }
            }
            Err(error) => override_with_evidence_error(report, "trace_finalization_failed", error),
        }
    }
    if opened {
        report.cleanup.attempted = true;
        match driver.close() {
            Ok(()) => report.cleanup.status = CleanupStatus::Passed,
            Err(error) => {
                report.cleanup.status = CleanupStatus::Failed;
                report.cleanup.error = Some(safe_driver_message(&error));
                override_with_evidence_error(report, "cleanup_failed", error);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_step(
    journey: &ValidatedJourney,
    run_root: &Path,
    driver: &mut dyn BrowserDriver,
    step_id: &str,
    index: usize,
    action: &ValidatedAction,
    has_navigated: bool,
    observation: &mut StepObservation,
    artifacts: &mut Vec<ArtifactRecord>,
) -> Result<StepResult, DriverError> {
    if has_navigated {
        let before = driver.current_url()?;
        observation.observed_url = Some(safe_url(&journey.origin, &before));
        if !journey.origin.contains(&before) {
            return Ok(StepResult::Blocked(format!(
                "observed URL is outside authorized origin {}",
                journey.origin
            )));
        }
    }

    let result = match action {
        ValidatedAction::Navigate { url } => {
            if !journey.origin.contains(url) {
                return Ok(StepResult::Blocked(
                    "commanded navigation is outside the authorized origin".to_owned(),
                ));
            }
            observation.expected_url = Some(safe_url(&journey.origin, url));
            match driver.navigate(url) {
                Ok(()) => {}
                Err(DriverError::NavigationBlocked(_)) => {
                    return Ok(StepResult::Blocked(
                        "agent-browser blocked navigation outside the authorized target".to_owned(),
                    ));
                }
                Err(error) => return Err(error),
            }
            let observed = driver.current_url()?;
            observation.observed_url = Some(safe_url(&journey.origin, &observed));
            if !journey.origin.contains(&observed) {
                StepResult::Blocked(format!(
                    "navigation reached outside authorized origin {}",
                    journey.origin
                ))
            } else {
                StepResult::Passed
            }
        }
        ValidatedAction::CheckUrl { url } => {
            observation.expected_url = Some(safe_url(&journey.origin, url));
            let observed = driver.current_url()?;
            observation.observed_url = Some(safe_url(&journey.origin, &observed));
            if !journey.origin.contains(&observed) {
                StepResult::Blocked(format!(
                    "observed URL is outside authorized origin {}",
                    journey.origin
                ))
            } else {
                let matched = observed == *url;
                observation.matched = Some(matched);
                if matched {
                    StepResult::Passed
                } else {
                    StepResult::Failed(
                        "current URL did not match the declared checkpoint".to_owned(),
                    )
                }
            }
        }
        ValidatedAction::CheckText {
            selector,
            expected,
            comparison,
        } => {
            let visible = driver.visible(selector)?;
            observation.visible = Some(visible);
            if !visible {
                let after = driver.current_url()?;
                observation.observed_url = Some(safe_url(&journey.origin, &after));
                if !journey.origin.contains(&after) {
                    return Ok(StepResult::Blocked(format!(
                        "visibility check ended outside authorized origin {}",
                        journey.origin
                    )));
                }
                return Ok(StepResult::Failed(
                    "declared visible-text target was not visible".to_owned(),
                ));
            }
            let actual = driver.text(selector)?;
            observation.observed_text_sha256 = Some(hex_digest(actual.as_bytes()));
            let matched = match comparison {
                TextComparison::Exact => actual == *expected,
                TextComparison::Contains => actual.contains(expected),
            };
            observation.matched = Some(matched);
            if matched {
                StepResult::Passed
            } else {
                StepResult::Failed("visible text did not match the declared checkpoint".to_owned())
            }
        }
        ValidatedAction::Capture { selector, alt_text } => {
            let visible = driver.visible(selector)?;
            observation.visible = Some(visible);
            if !visible {
                let after = driver.current_url()?;
                observation.observed_url = Some(safe_url(&journey.origin, &after));
                if !journey.origin.contains(&after) {
                    return Ok(StepResult::Blocked(format!(
                        "capture visibility check ended outside authorized origin {}",
                        journey.origin
                    )));
                }
                return Ok(StepResult::Failed(
                    "focused capture target was not visible".to_owned(),
                ));
            }
            let stem = format!("{:03}-{}", index + 1, step_id);
            let raw = run_root.join("evidence").join(format!("{stem}.raw.png"));
            let focused = run_root
                .join("evidence")
                .join(format!("{stem}.focused.png"));
            let metadata = run_root
                .join("evidence")
                .join(format!("{stem}.focused.json"));
            let capture = driver.capture(selector, &raw)?;
            observation.target_box_css = Some(capture.target);
            observation.viewport = Some(capture.viewport);
            observation.capture_token = Some(capture.capture_token.clone());
            observation.box_command_sequence = Some(capture.box_command_sequence);
            observation.screenshot_command_sequence = Some(capture.screenshot_command_sequence);
            let raw = capture.raw_path;
            let raw_artifact = artifact_record(
                run_root,
                &raw,
                "raw_screenshot",
                "image/png",
                Some(step_id),
                None,
            )?;
            let raw_path = raw_artifact.path.clone();
            let raw_sha256 = raw_artifact.sha256.clone();
            observation.artifact_path = Some(raw_path.clone());
            artifacts.push(raw_artifact);
            let focus = focus::render(FocusRequest {
                run_root,
                raw_path: &raw,
                focused_path: &focused,
                metadata_path: &metadata,
                capture_step_id: step_id,
                capture_token: &capture.capture_token,
                box_command_sequence: capture.box_command_sequence,
                screenshot_command_sequence: capture.screenshot_command_sequence,
                alt_text,
                expected_source_sha256: &raw_sha256,
                target: capture.target,
                viewport: capture.viewport,
            })
            .map_err(|error| DriverError::Artifact(error.to_string()))?;
            artifacts.push(artifact_record(
                run_root,
                &focused,
                "focused_screenshot",
                "image/png",
                Some(step_id),
                Some(&raw_path),
            )?);
            artifacts.push(artifact_record(
                run_root,
                &metadata,
                "focus_metadata",
                "application/json",
                Some(step_id),
                Some(&raw_path),
            )?);
            observation.artifact_path = Some(focus.metadata.derivative.path);
            StepResult::Passed
        }
    };

    if !matches!(result, StepResult::Blocked(_)) {
        let after = driver.current_url()?;
        observation.observed_url = Some(safe_url(&journey.origin, &after));
        if !journey.origin.contains(&after) {
            return Ok(StepResult::Blocked(format!(
                "step ended outside authorized origin {}",
                journey.origin
            )));
        }
    }
    Ok(result)
}

enum StepResult {
    Passed,
    Failed(String),
    Blocked(String),
}

fn artifact_record(
    root: &Path,
    path: &Path,
    kind: &'static str,
    media_type: &'static str,
    step_id: Option<&str>,
    source: Option<&str>,
) -> Result<ArtifactRecord, DriverError> {
    let root = root
        .canonicalize()
        .map_err(|error| DriverError::Artifact(error.to_string()))?;
    let path = path
        .canonicalize()
        .map_err(|error| DriverError::Artifact(error.to_string()))?;
    if !path.starts_with(&root) {
        return Err(DriverError::Artifact(
            "artifact path escapes the run directory".to_owned(),
        ));
    }
    let metadata = fs::metadata(&path).map_err(|error| DriverError::Artifact(error.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(DriverError::Artifact(
            "artifact is not a non-empty regular file".to_owned(),
        ));
    }
    let sha256 = file_sha256(&path)?;
    let relative = path
        .strip_prefix(&root)
        .expect("path containment was checked")
        .to_string_lossy()
        .replace('\\', "/");
    Ok(ArtifactRecord {
        kind,
        path: relative,
        size_bytes: metadata.len(),
        media_type,
        sha256,
        step_id: step_id.map(ToOwned::to_owned),
        source_artifact: source.map(ToOwned::to_owned),
    })
}

fn file_sha256(path: &Path) -> Result<String, DriverError> {
    let mut file =
        fs::File::open(path).map_err(|error| DriverError::Artifact(error.to_string()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| DriverError::Artifact(error.to_string()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn create_run_directory(base: &Path) -> Result<(PathBuf, String), String> {
    fs::create_dir_all(base).map_err(|error| error.to_string())?;
    let directory = tempfile::Builder::new()
        .prefix("crawlson-run-")
        .tempdir_in(base)
        .map_err(|error| error.to_string())?
        .keep();
    let directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if directory.to_str().is_none() {
        return Err("run directory path must be valid UTF-8".to_owned());
    }
    let run_id = directory
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("crawlson-run")
        .trim_start_matches("crawlson-run-")
        .to_owned();
    fs::create_dir_all(directory.join("evidence")).map_err(|error| error.to_string())?;
    Ok((directory, run_id))
}

fn base_report(root: &Path, run_id: &str, source_path: String, started_at: u64) -> RunReport {
    RunReport {
        schema_version: 1,
        crawlson_version: VERSION,
        run_id: run_id.to_owned(),
        run_directory: root.to_string_lossy().to_string(),
        journey: JourneyProvenance {
            source_path,
            source_sha256: None,
            id: None,
            revision: None,
        },
        target_origin: None,
        started_at_unix_ms: started_at,
        finished_at_unix_ms: started_at,
        duration_ms: 0,
        outcome: RunOutcome::Error,
        execution_outcome: RunOutcome::Error,
        reason: Reason {
            code: "run_incomplete".to_owned(),
            message: "run did not complete".to_owned(),
        },
        execution_reason: Reason {
            code: "run_incomplete".to_owned(),
            message: "run did not complete".to_owned(),
        },
        driver: DriverReport {
            name: "agent-browser",
            version: None,
            session: None,
            commands: Vec::new(),
        },
        steps: Vec::new(),
        artifacts: Vec::new(),
        diagnostics: None,
        cleanup: CleanupReport {
            attempted: false,
            status: CleanupStatus::NotNeeded,
            error: None,
        },
    }
}

fn empty_report(
    source_path: String,
    started_at: u64,
    timer: Instant,
    code: &str,
    message: String,
) -> RunReport {
    let mut report = base_report(Path::new(""), "unavailable", source_path, started_at);
    report.run_directory.clear();
    finish_error(&mut report, code, message);
    report.finished_at_unix_ms = unix_ms();
    report.duration_ms = elapsed_ms(timer);
    report
}

fn finalize(mut report: RunReport, timer: Instant, root: &Path) -> RunReport {
    report.finished_at_unix_ms = unix_ms();
    report.duration_ms = elapsed_ms(timer);
    if let Err(error) = write_report(root, &report) {
        report.outcome = RunOutcome::Error;
        report.reason = Reason {
            code: "report_write_failed".to_owned(),
            message: error,
        };
    }
    report
}

fn write_report(root: &Path, report: &RunReport) -> Result<(), String> {
    let path = root.join("report.json");
    let mut file = AtomicWriteFile::open(&path).map_err(|error| error.to_string())?;
    serde_json::to_writer_pretty(&mut file, report).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    file.commit().map_err(|error| error.to_string())
}

fn finish_error(report: &mut RunReport, code: &str, message: String) {
    report.outcome = RunOutcome::Error;
    report.execution_outcome = RunOutcome::Error;
    report.reason = Reason {
        code: code.to_owned(),
        message: message.clone(),
    };
    report.execution_reason = Reason {
        code: code.to_owned(),
        message,
    };
}

fn finish_blocked(report: &mut RunReport, code: &str, message: String) {
    report.outcome = RunOutcome::Blocked;
    report.execution_outcome = RunOutcome::Blocked;
    report.reason = Reason {
        code: code.to_owned(),
        message: message.clone(),
    };
    report.execution_reason = Reason {
        code: code.to_owned(),
        message,
    };
}

fn set_driver_error(outcome: &mut RunOutcome, reason: &mut Reason, error: DriverError) {
    *outcome = if matches!(error, DriverError::NavigationBlocked(_)) {
        RunOutcome::Blocked
    } else {
        RunOutcome::Error
    };
    *reason = Reason {
        code: driver_error_code(&error).to_owned(),
        message: safe_driver_message(&error),
    };
}

fn override_with_evidence_error(report: &mut RunReport, code: &str, error: DriverError) {
    override_with_message(report, code, safe_driver_message(&error));
}

fn override_with_message(report: &mut RunReport, code: &str, message: String) {
    report.outcome = RunOutcome::Error;
    report.reason = Reason {
        code: code.to_owned(),
        message,
    };
}

fn driver_error_code(error: &DriverError) -> &'static str {
    match error {
        DriverError::Unavailable(_) => "driver_unavailable",
        DriverError::Timeout { .. } => "driver_timeout",
        DriverError::OutputLimit(_) => "driver_output_limit",
        DriverError::CommandFailed { .. } => "driver_command_failed",
        DriverError::NavigationBlocked(_) => "origin_not_authorized",
        DriverError::Protocol { .. } => "driver_protocol",
        DriverError::Artifact(_) => "artifact_invalid",
        DriverError::Io(_) => "driver_io",
    }
}

fn safe_driver_message(error: &DriverError) -> String {
    match error {
        DriverError::Unavailable(_) => "agent-browser is unavailable".to_owned(),
        DriverError::Timeout {
            capability,
            seconds,
        } => format!("agent-browser command '{capability}' timed out after {seconds} seconds"),
        DriverError::OutputLimit(capability) => {
            format!("agent-browser command '{capability}' exceeded its output limit")
        }
        DriverError::CommandFailed { capability, .. } => {
            format!("agent-browser command '{capability}' failed")
        }
        DriverError::NavigationBlocked(_) => {
            "agent-browser blocked navigation outside the authorized target".to_owned()
        }
        DriverError::Protocol { capability, .. } => {
            format!("agent-browser returned an invalid response for '{capability}'")
        }
        DriverError::Artifact(_) => "browser evidence artifact was invalid".to_owned(),
        DriverError::Io(_) => "agent-browser I/O failed".to_owned(),
    }
}

fn safe_source_label(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("journey.toml")
        .to_owned()
}

fn safe_url(origin: &Origin, url: &url::Url) -> String {
    if !origin.contains(url) {
        return "unauthorized-origin".to_owned();
    }
    let mut safe = url.clone();
    let _ = safe.set_username("");
    let _ = safe.set_password(None);
    safe.set_query(None);
    safe.set_fragment(None);
    safe.to_string()
}

fn action_kind(action: &ValidatedAction) -> &'static str {
    match action {
        ValidatedAction::Navigate { .. } => "navigate",
        ValidatedAction::CheckUrl { .. } => "check_url",
        ValidatedAction::CheckText { .. } => "check_text",
        ValidatedAction::Capture { .. } => "capture",
    }
}

fn session_name(run_id: &str) -> String {
    let suffix: String = run_id
        .bytes()
        .filter(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        .take(24)
        .map(char::from)
        .collect();
    format!(
        "crawlson-{}",
        if suffix.is_empty() { "run" } else { &suffix }
    )
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn elapsed_ms(timer: Instant) -> u64 {
    u64::try_from(timer.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::DiagnosticsSummary;
    use crate::journey::{
        EvidencePolicy, JourneyMeta, JourneyMode, ValidatedAction, ValidatedStep,
    };
    use url::Url;

    struct ScriptedDriver {
        url: Url,
        text: String,
        fail_close: bool,
        records: Vec<DriverCommandRecord>,
    }

    impl BrowserDriver for ScriptedDriver {
        fn prepare(&mut self) -> Result<(), DriverError> {
            Ok(())
        }
        fn start_trace(&mut self) -> Result<(), DriverError> {
            Ok(())
        }
        fn navigate(&mut self, url: &Url) -> Result<(), DriverError> {
            self.url = url.clone();
            Ok(())
        }
        fn current_url(&mut self) -> Result<Url, DriverError> {
            Ok(self.url.clone())
        }
        fn text(&mut self, _selector: &str) -> Result<String, DriverError> {
            Ok(self.text.clone())
        }
        fn visible(&mut self, _selector: &str) -> Result<bool, DriverError> {
            Ok(true)
        }
        fn capture(
            &mut self,
            _selector: &str,
            _path: &Path,
        ) -> Result<crate::driver::CaptureBundle, DriverError> {
            unreachable!()
        }
        fn diagnostics(&mut self) -> Result<DiagnosticsSummary, DriverError> {
            Ok(DiagnosticsSummary {
                console_messages: 0,
                console_sha256: hex_digest(b"[]"),
                page_errors: 0,
                page_errors_sha256: hex_digest(b"[]"),
            })
        }
        fn stop_trace(&mut self, path: &Path) -> Result<PathBuf, DriverError> {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"{\"traceEvents\":[]}").unwrap();
            Ok(path.to_path_buf())
        }
        fn close(&mut self) -> Result<(), DriverError> {
            if self.fail_close {
                Err(DriverError::CommandFailed {
                    capability: "close".to_owned(),
                    message: "fixture close failed".to_owned(),
                })
            } else {
                Ok(())
            }
        }
        fn records(&self) -> Vec<DriverCommandRecord> {
            self.records.clone()
        }
    }

    fn validated(expected: &str) -> ValidatedJourney {
        ValidatedJourney {
            source_path: PathBuf::from("fixture.toml"),
            source_sha256: hex_digest(b"fixture"),
            meta: JourneyMeta {
                id: "demo".to_owned(),
                revision: 1,
                title: "Demo".to_owned(),
                purpose: "Demo".to_owned(),
                expected_outcome: "Demo".to_owned(),
                mode: JourneyMode::ReadOnly,
            },
            origin: Origin::parse("http://127.0.0.1:4173").unwrap(),
            authentication: None,
            evidence: EvidencePolicy {
                trace: true,
                diagnostics: true,
            },
            steps: vec![
                ValidatedStep {
                    id: "open".to_owned(),
                    title: "Open".to_owned(),
                    guide_instruction: None,
                    evidence_for: Vec::new(),
                    action: ValidatedAction::Navigate {
                        url: Url::parse("http://127.0.0.1:4173/").unwrap(),
                    },
                },
                ValidatedStep {
                    id: "check".to_owned(),
                    title: "Check".to_owned(),
                    guide_instruction: None,
                    evidence_for: Vec::new(),
                    action: ValidatedAction::CheckText {
                        selector: "h1".to_owned(),
                        expected: expected.to_owned(),
                        comparison: TextComparison::Exact,
                    },
                },
            ],
        }
    }

    fn test_report(root: &Path) -> RunReport {
        base_report(root, "fixture", "fixture.toml".to_owned(), unix_ms())
    }

    #[test]
    fn deterministic_false_is_failed_and_cleanup_failure_is_error() {
        let directory = tempfile::tempdir().unwrap();
        let mut driver = ScriptedDriver {
            url: Url::parse("about:blank").unwrap(),
            text: "Different".to_owned(),
            fail_close: false,
            records: Vec::new(),
        };
        let mut report = test_report(directory.path());
        execute(
            &validated("Expected"),
            directory.path(),
            &mut driver,
            &mut report,
        );
        assert_eq!(report.outcome, RunOutcome::Failed);
        assert_eq!(report.reason.code, "checkpoint_failed");
        assert_eq!(report.cleanup.status, CleanupStatus::Passed);

        let mut driver = ScriptedDriver {
            url: Url::parse("about:blank").unwrap(),
            text: "Expected".to_owned(),
            fail_close: true,
            records: Vec::new(),
        };
        let mut report = test_report(directory.path());
        execute(
            &validated("Expected"),
            directory.path(),
            &mut driver,
            &mut report,
        );
        assert_eq!(report.execution_outcome, RunOutcome::Passed);
        assert_eq!(report.outcome, RunOutcome::Error);
        assert_eq!(report.reason.code, "cleanup_failed");
    }

    #[test]
    fn observed_redirect_outside_origin_is_blocked() {
        struct RedirectDriver(ScriptedDriver);
        impl BrowserDriver for RedirectDriver {
            fn prepare(&mut self) -> Result<(), DriverError> {
                Ok(())
            }
            fn start_trace(&mut self) -> Result<(), DriverError> {
                Ok(())
            }
            fn navigate(&mut self, _url: &Url) -> Result<(), DriverError> {
                self.0.url = Url::parse("http://127.0.0.1:9999/private").unwrap();
                Ok(())
            }
            fn current_url(&mut self) -> Result<Url, DriverError> {
                Ok(self.0.url.clone())
            }
            fn text(&mut self, selector: &str) -> Result<String, DriverError> {
                self.0.text(selector)
            }
            fn visible(&mut self, selector: &str) -> Result<bool, DriverError> {
                self.0.visible(selector)
            }
            fn capture(
                &mut self,
                selector: &str,
                path: &Path,
            ) -> Result<crate::driver::CaptureBundle, DriverError> {
                self.0.capture(selector, path)
            }
            fn diagnostics(&mut self) -> Result<DiagnosticsSummary, DriverError> {
                self.0.diagnostics()
            }
            fn stop_trace(&mut self, path: &Path) -> Result<PathBuf, DriverError> {
                self.0.stop_trace(path)
            }
            fn close(&mut self) -> Result<(), DriverError> {
                self.0.close()
            }
            fn records(&self) -> Vec<DriverCommandRecord> {
                Vec::new()
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let mut driver = RedirectDriver(ScriptedDriver {
            url: Url::parse("about:blank").unwrap(),
            text: "Expected".to_owned(),
            fail_close: false,
            records: Vec::new(),
        });
        let mut report = test_report(directory.path());
        execute(
            &validated("Expected"),
            directory.path(),
            &mut driver,
            &mut report,
        );
        assert_eq!(report.execution_outcome, RunOutcome::Blocked);
        assert_eq!(report.reason.code, "origin_not_authorized");
        assert_eq!(report.steps.len(), 1);
    }
}
