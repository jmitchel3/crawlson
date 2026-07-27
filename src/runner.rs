use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::auth::{AGENT_BROWSER_STATE_FILE_PROVIDER, ValidatedState};
use crate::doctor::{self, CheckStatus, DoctorOptions};
use crate::driver::{
    AgentBrowserDriver, BrowserDriver, CaptureBundle, DiagnosticsSummary, DriverCommandRecord,
    DriverError, DriverPolicyMode,
};
use crate::focus::{self, CssBox, FocusRequest, Viewport};
use crate::journey::{self, Origin, TextComparison, ValidatedAction, ValidatedJourney, hex_digest};
use crate::journey::{MutationValue, StepEffect, StepPhase};
use crate::recovery::{RecoveryRecord, RecoveryStore};
use crate::{CommandResult, VERSION};

pub const EXIT_PASSED: u8 = 0;
pub const EXIT_FAILED: u8 = 1;
pub const EXIT_BLOCKED: u8 = 3;
pub const EXIT_ERROR: u8 = 4;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub journey_path: PathBuf,
    pub allowed_origin: Option<String>,
    pub allowed_actions: Vec<String>,
    pub allowed_mutations: Vec<String>,
    pub allowed_production_mutations: Vec<String>,
    pub auth_state: Option<PathBuf>,
    pub output_directory: PathBuf,
    pub agent_browser: Option<PathBuf>,
    pub browser_executable: Option<PathBuf>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_authorization: Option<ActionAuthorizationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_authorization: Option<MutationAuthorizationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationReport>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture: Option<FixtureLifecycleReport>,
    pub cleanup: CleanupReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionAuthorizationReport {
    pub required: Vec<String>,
    pub granted: Vec<String>,
    pub binding_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutationAuthorizationReport {
    pub required: Vec<String>,
    pub granted: Vec<String>,
    pub production_required: bool,
    pub production_granted: Vec<String>,
    pub binding_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixtureLifecycleReport {
    pub kind: &'static str,
    pub maximum_lifetime_seconds: u64,
    pub setup_status: FixtureSetupStatus,
    pub mutation_attempted: bool,
    pub cleanup_status: FixtureCleanupStatus,
    pub recovery_required: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FixtureSetupStatus {
    NotStarted,
    Passed,
    Failed,
    Blocked,
    Error,
    EffectUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FixtureCleanupStatus {
    NotNeeded,
    Passed,
    Failed,
    EffectUnknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticationReport {
    pub provider: String,
    pub role: String,
    pub verification_step: String,
    pub status: AuthenticationStatus,
    pub binding_sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationStatus {
    Missing,
    Unsupported,
    Invalid,
    LoadFailed,
    Blocked,
    Verified,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<StepPhase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<StepEffect>,
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
    pub enabled: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_state: Option<ActionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_grant_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard_command_sequence: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_command_sequence: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_href: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    NotAttempted,
    DriverAcknowledged,
    EffectVerified,
    EffectUnverified,
    EffectUnknown,
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
    report.schema_version = match journey.schema_version {
        5 => 4,
        4 => 3,
        3 => 2,
        _ => 1,
    };
    if matches!(journey.schema_version, 4 | 5) {
        let authentication = journey
            .authentication
            .as_ref()
            .expect("journey v4 authentication was validated");
        let verification_step = authentication
            .verification_step
            .as_ref()
            .expect("journey v4 verification_step was validated");
        report.authentication = Some(AuthenticationReport {
            provider: authentication.provider.clone(),
            role: authentication.role.clone(),
            verification_step: verification_step.clone(),
            status: AuthenticationStatus::Blocked,
            binding_sha256: hex_digest(authentication_binding(&journey).as_bytes()),
        });
    }
    if journey.schema_version == 5 {
        let fixture = journey
            .fixture
            .as_ref()
            .expect("journey v5 fixture was validated");
        report.fixture = Some(FixtureLifecycleReport {
            kind: "self_expiring_ui",
            maximum_lifetime_seconds: fixture.maximum_lifetime_seconds,
            setup_status: FixtureSetupStatus::NotStarted,
            mutation_attempted: false,
            cleanup_status: FixtureCleanupStatus::NotNeeded,
            recovery_required: false,
        });
    }
    // Build action provenance before the other fail-closed preconditions so a
    // v3 report remains structurally renderable even when target authorization
    // or authentication blocks execution first. Preserve those earlier safety
    // reasons by applying the action-authorization result only afterwards.
    let action_authorization = authorize_actions(
        &journey,
        &options.allowed_actions,
        &mut report.action_authorization,
    );
    let mutation_authorization = authorize_mutations(
        &journey,
        &options.allowed_mutations,
        &options.allowed_production_mutations,
        &mut report.mutation_authorization,
    );

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
    if journey.schema_version < 4 && journey.authentication.is_some() {
        finish_blocked(
            &mut report,
            "authentication_unavailable",
            "this journey requires authentication, but no authentication adapter is available"
                .to_owned(),
        );
        return finalize(report, overall_start, &run_root);
    }

    if let Err((code, message)) = action_authorization {
        finish_blocked(&mut report, code, message);
        return finalize(report, overall_start, &run_root);
    }
    if let Err((code, message)) = mutation_authorization {
        finish_blocked(&mut report, code, message);
        return finalize(report, overall_start, &run_root);
    }

    let (recovery_store, pending_recovery) = if journey.schema_version == 5 {
        let store = match RecoveryStore::global(&options.output_directory) {
            Ok(store) => store,
            Err(error) => {
                finish_blocked(&mut report, error.code(), error.to_string());
                return finalize(report, overall_start, &run_root);
            }
        };
        match store.check_pending(&journey.origin) {
            Ok(None) => (Some(store), None),
            Ok(Some(record)) => {
                if let Some(fixture) = &mut report.fixture {
                    fixture.recovery_required = true;
                }
                let cleanup_step_ids = journey
                    .cleanup_steps
                    .iter()
                    .map(|step| step.id.clone())
                    .collect::<Vec<_>>();
                if record.journey_id != journey.meta.id
                    || record.revision != journey.meta.revision
                    || record.source_sha256 != journey.source_sha256
                    || record.target_origin != journey.origin.to_string()
                    || record.cleanup_step_ids != cleanup_step_ids
                {
                    finish_blocked(
                        &mut report,
                        "recovery_pending_mismatch",
                        "a prior mutation for this exact origin requires its exact original journey and cleanup contract"
                            .to_owned(),
                    );
                    return finalize(report, overall_start, &run_root);
                }
                (Some(store), Some(record))
            }
            Err(error) => {
                finish_blocked(&mut report, error.code(), error.to_string());
                return finalize(report, overall_start, &run_root);
            }
        }
    } else {
        (None, None)
    };

    let authentication_state = if matches!(journey.schema_version, 4 | 5) {
        let authentication = journey
            .authentication
            .as_ref()
            .expect("journey v4 authentication was validated");
        if authentication.provider != AGENT_BROWSER_STATE_FILE_PROVIDER {
            set_authentication_status(&mut report, AuthenticationStatus::Unsupported);
            finish_blocked(
                &mut report,
                "authentication_provider_unsupported",
                "the declared authentication provider is not supported".to_owned(),
            );
            return finalize(report, overall_start, &run_root);
        }
        let Some(path) = options.auth_state.as_deref() else {
            set_authentication_status(&mut report, AuthenticationStatus::Missing);
            finish_blocked(
                &mut report,
                "authentication_state_missing",
                "this journey requires an external agent-browser authentication state file"
                    .to_owned(),
            );
            return finalize(report, overall_start, &run_root);
        };
        match ValidatedState::load(path, &journey.origin) {
            Ok(state) => Some(state),
            Err(_) => {
                set_authentication_status(&mut report, AuthenticationStatus::Invalid);
                finish_blocked(
                    &mut report,
                    "authentication_state_invalid",
                    "the supplied authentication state did not satisfy the bounded same-origin state-file contract"
                        .to_owned(),
                );
                return finalize(report, overall_start, &run_root);
            }
        }
    } else {
        if options.auth_state.is_some() {
            finish_blocked(
                &mut report,
                "authentication_unexpected",
                "authentication state was supplied for a journey without an executable authentication contract"
                    .to_owned(),
            );
            return finalize(report, overall_start, &run_root);
        }
        None
    };

    if journey.schema_version == 5 && options.browser_executable.is_none() {
        finish_blocked(
            &mut report,
            "extension_browser_missing",
            "mutating journeys require --browser-executable pointing to Chromium or Chrome for Testing"
                .to_owned(),
        );
        return finalize(report, overall_start, &run_root);
    }
    if journey.schema_version != 5 && options.browser_executable.is_some() {
        finish_blocked(
            &mut report,
            "extension_browser_unexpected",
            "--browser-executable is accepted only for a mutating journey".to_owned(),
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
    let has_link_action = journey
        .phase_steps()
        .any(|(_, step)| matches!(step.action, ValidatedAction::FollowLink { .. }));
    let has_mutation = journey.mutating_steps().next().is_some();
    let policy_mode = match (
        matches!(journey.schema_version, 4 | 5),
        has_link_action,
        has_mutation,
    ) {
        (false, false, false) => DriverPolicyMode::ReadOnly,
        (false, true, false) => DriverPolicyMode::FollowLink,
        (true, false, false) => DriverPolicyMode::AuthenticatedReadOnly,
        (true, true, false) => DriverPolicyMode::AuthenticatedFollowLink,
        (false, false, true) => DriverPolicyMode::Mutation,
        (true, false, true) => DriverPolicyMode::AuthenticatedMutation,
        _ => {
            finish_error(
                &mut report,
                "journey_invalid",
                "unsupported combination of browser action capabilities".to_owned(),
            );
            return finalize(report, overall_start, &run_root);
        }
    };
    let mut driver = match AgentBrowserDriver::new_with_policy_mode_and_browser(
        executable,
        &run_root,
        journey.origin.clone(),
        session,
        options.action_timeout,
        options.run_timeout,
        policy_mode,
        options.browser_executable.as_deref(),
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

    if journey.schema_version == 5 {
        execute_mutating(
            &journey,
            &run_root,
            &run_id,
            &mut driver,
            authentication_state,
            recovery_store.expect("v5 recovery store was initialized"),
            pending_recovery,
            &mut report,
        );
    } else {
        execute(
            &journey,
            &run_root,
            &mut driver,
            authentication_state,
            &mut report,
        );
    }
    report.driver.commands = driver.records();
    finalize(report, overall_start, &run_root)
}

fn authorize_actions(
    journey: &ValidatedJourney,
    supplied: &[String],
    report: &mut Option<ActionAuthorizationReport>,
) -> Result<(), (&'static str, String)> {
    let required = journey
        .steps
        .iter()
        .filter(|step| matches!(step.action, ValidatedAction::FollowLink { .. }))
        .map(|step| action_grant(journey, &step.id))
        .collect::<BTreeSet<_>>();
    if supplied.iter().any(|grant| !valid_action_grant(grant)) {
        if journey.schema_version >= 3 || !required.is_empty() {
            let required_values = required.iter().cloned().collect::<Vec<_>>();
            let binding = action_authorization_binding(journey, &required_values, &[]);
            *report = Some(ActionAuthorizationReport {
                required: required_values,
                granted: Vec::new(),
                binding_sha256: hex_digest(binding.as_bytes()),
            });
        }
        return Err((
            "action_authorization_invalid",
            "action authorization grant syntax is invalid".to_owned(),
        ));
    }
    let granted = supplied.iter().cloned().collect::<BTreeSet<_>>();
    let duplicates = granted.len() != supplied.len();

    if required.is_empty() {
        if journey.schema_version >= 3 {
            let binding = action_authorization_binding(journey, &[], &[]);
            *report = Some(ActionAuthorizationReport {
                required: Vec::new(),
                granted: Vec::new(),
                binding_sha256: hex_digest(binding.as_bytes()),
            });
        }
        return if granted.is_empty() {
            Ok(())
        } else {
            Err((
                "action_authorization_unexpected",
                "this journey declares no interactive action to authorize".to_owned(),
            ))
        };
    }

    let required_values = required.iter().cloned().collect::<Vec<_>>();
    let granted_values = granted.iter().cloned().collect::<Vec<_>>();
    let binding = action_authorization_binding(journey, &required_values, &granted_values);
    *report = Some(ActionAuthorizationReport {
        required: required_values,
        granted: granted_values,
        binding_sha256: hex_digest(binding.as_bytes()),
    });

    if duplicates {
        return Err((
            "action_authorization_invalid",
            "action authorization grants must not be duplicated".to_owned(),
        ));
    }
    if required != granted {
        let missing = required.difference(&granted).cloned().collect::<Vec<_>>();
        let message = if !missing.is_empty() {
            format!(
                "explicit action authorization is required; rerun with {}",
                missing
                    .iter()
                    .map(|grant| format!("--allow-action {grant}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        } else {
            "unexpected action authorization grant set".to_owned()
        };
        return Err(("action_authorization_mismatch", message));
    }
    Ok(())
}

fn action_authorization_binding(
    journey: &ValidatedJourney,
    required: &[String],
    granted: &[String],
) -> String {
    format!(
        "crawlson-action-grant-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nrequired={}\ngranted={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        required.join(","),
        granted.join(",")
    )
}

fn authorize_mutations(
    journey: &ValidatedJourney,
    supplied: &[String],
    supplied_production: &[String],
    report: &mut Option<MutationAuthorizationReport>,
) -> Result<(), (&'static str, String)> {
    let required = journey
        .mutating_steps()
        .map(|step| action_grant(journey, &step.id))
        .collect::<BTreeSet<_>>();
    let production_required = !required.is_empty() && !journey.origin.is_literal_loopback();
    let syntax_valid = supplied.iter().all(|grant| valid_action_grant(grant))
        && supplied_production
            .iter()
            .all(|grant| valid_action_grant(grant));
    let granted = supplied.iter().cloned().collect::<BTreeSet<_>>();
    let production_granted = supplied_production.iter().cloned().collect::<BTreeSet<_>>();
    let required_values = required.iter().cloned().collect::<Vec<_>>();
    let granted_values = granted.iter().cloned().collect::<Vec<_>>();
    let production_values = production_granted.iter().cloned().collect::<Vec<_>>();
    if journey.schema_version == 5 || !supplied.is_empty() || !supplied_production.is_empty() {
        let binding = mutation_authorization_binding(
            journey,
            &required_values,
            &granted_values,
            production_required,
            &production_values,
        );
        *report = Some(MutationAuthorizationReport {
            required: required_values.clone(),
            granted: granted_values,
            production_required,
            production_granted: production_values,
            binding_sha256: hex_digest(binding.as_bytes()),
        });
    }
    if !syntax_valid
        || granted.len() != supplied.len()
        || production_granted.len() != supplied_production.len()
    {
        return Err((
            "mutation_authorization_invalid",
            "mutation authorization grants must be valid and unique".to_owned(),
        ));
    }
    if required.is_empty() {
        return if granted.is_empty() && production_granted.is_empty() {
            Ok(())
        } else {
            Err((
                "mutation_authorization_unexpected",
                "this journey declares no mutation to authorize".to_owned(),
            ))
        };
    }
    if granted != required {
        let missing = required.difference(&granted).cloned().collect::<Vec<_>>();
        let message = if missing.is_empty() {
            "unexpected mutation authorization grant set".to_owned()
        } else {
            format!(
                "explicit mutation authorization is required; rerun with {}",
                missing
                    .iter()
                    .map(|grant| format!("--allow-mutation {grant}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        return Err(("mutation_authorization_mismatch", message));
    }
    if production_required && production_granted != required {
        let missing = required
            .difference(&production_granted)
            .cloned()
            .collect::<Vec<_>>();
        let message = if missing.is_empty() {
            "unexpected production mutation authorization grant set".to_owned()
        } else {
            format!(
                "non-loopback mutations require an extra exact production confirmation; rerun with {}",
                missing
                    .iter()
                    .map(|grant| format!("--allow-production-mutation {grant}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        return Err(("production_mutation_authorization_mismatch", message));
    }
    if !production_required && !production_granted.is_empty() {
        return Err((
            "production_mutation_authorization_unexpected",
            "literal-loopback mutations must not carry a production mutation grant".to_owned(),
        ));
    }
    Ok(())
}

fn mutation_authorization_binding(
    journey: &ValidatedJourney,
    required: &[String],
    granted: &[String],
    production_required: bool,
    production_granted: &[String],
) -> String {
    format!(
        "crawlson-mutation-grant-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nrequired={}\ngranted={}\nproduction_required={}\nproduction_granted={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        required.join(","),
        granted.join(","),
        production_required,
        production_granted.join(",")
    )
}

fn authentication_binding(journey: &ValidatedJourney) -> String {
    let authentication = journey
        .authentication
        .as_ref()
        .expect("authentication binding requires a declaration");
    format!(
        "crawlson-auth-requirement-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nprovider={}\nrole={}\nverification_step={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        authentication.provider,
        authentication.role,
        authentication
            .verification_step
            .as_deref()
            .unwrap_or_default()
    )
}

fn set_authentication_status(report: &mut RunReport, status: AuthenticationStatus) {
    if let Some(authentication) = &mut report.authentication {
        authentication.status = status;
    }
}

fn valid_action_grant(value: &str) -> bool {
    let Some((journey_id, remainder)) = value.split_once('@') else {
        return false;
    };
    let Some((revision, step_id)) = remainder.split_once(':') else {
        return false;
    };
    valid_action_identifier(journey_id)
        && valid_action_identifier(step_id)
        && revision
            .parse::<u64>()
            .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == revision)
}

fn valid_action_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn action_grant(journey: &ValidatedJourney, step_id: &str) -> String {
    format!("{}@{}:{}", journey.meta.id, journey.meta.revision, step_id)
}

#[allow(clippy::too_many_arguments)]
fn execute_mutating(
    journey: &ValidatedJourney,
    run_root: &Path,
    run_id: &str,
    driver: &mut dyn BrowserDriver,
    authentication_state: Option<ValidatedState>,
    recovery_store: RecoveryStore,
    pending_recovery: Option<RecoveryRecord>,
    report: &mut RunReport,
) {
    let interrupted = match mutation_interrupt_flag() {
        Ok(flag) => flag,
        Err(message) => {
            finish_error(report, "signal_handler_unavailable", message);
            return;
        }
    };
    let mut prepared = false;
    let mut trace_started = false;
    let mut primary = RunOutcome::Passed;
    let mut primary_reason = Reason {
        code: "journey_passed".to_owned(),
        message:
            "all setup, visible mutations, deterministic checks, evidence, and fixture cleanup completed"
                .to_owned(),
    };

    match driver.prepare() {
        Ok(()) => prepared = true,
        Err(error) => set_driver_error(&mut primary, &mut primary_reason, error),
    }
    if primary == RunOutcome::Passed {
        let Some(authentication_state) = authentication_state else {
            set_authentication_status(report, AuthenticationStatus::Missing);
            primary = RunOutcome::Blocked;
            primary_reason = Reason {
                code: "authentication_state_missing".to_owned(),
                message: "mutating journeys require disposable authenticated state".to_owned(),
            };
            finish_mutating_session(
                journey,
                run_root,
                driver,
                prepared,
                trace_started,
                primary,
                primary_reason,
                report,
            );
            return;
        };
        match authentication_state.stage() {
            Ok(staged) => {
                let load_result = driver.load_authentication(staged.path());
                let cleanup_result = staged.close();
                if load_result.is_err() || cleanup_result.is_err() {
                    set_authentication_status(report, AuthenticationStatus::LoadFailed);
                    primary = RunOutcome::Error;
                    primary_reason = Reason {
                        code: "authentication_state_load_failed".to_owned(),
                        message:
                            "agent-browser could not safely load disposable authentication state"
                                .to_owned(),
                    };
                }
            }
            Err(_) => {
                set_authentication_status(report, AuthenticationStatus::LoadFailed);
                primary = RunOutcome::Error;
                primary_reason = Reason {
                    code: "authentication_state_load_failed".to_owned(),
                    message: "agent-browser could not safely stage disposable authentication state"
                        .to_owned(),
                };
            }
        }
        drop(authentication_state);
    }
    if primary == RunOutcome::Passed {
        match driver.start_trace() {
            Ok(()) => trace_started = true,
            Err(error) => set_driver_error(&mut primary, &mut primary_reason, error),
        }
    }

    let fixture_token = fixture_token(run_id);
    let mut has_navigated = false;
    let mut sequence = 0usize;
    if primary == RunOutcome::Passed {
        let setup_limit = pending_recovery
            .as_ref()
            .map_or(journey.setup_steps.len(), |_| {
                let verification_step = journey
                    .authentication
                    .as_ref()
                    .and_then(|authentication| authentication.verification_step.as_deref())
                    .expect("v5 authentication verification was validated");
                journey
                    .setup_steps
                    .iter()
                    .position(|step| step.id == verification_step)
                    .expect("v5 authentication verification is a setup step")
                    + 1
            });
        for step in &journey.setup_steps[..setup_limit] {
            if interrupted.load(Ordering::Acquire) {
                primary = RunOutcome::Error;
                primary_reason = Reason {
                    code: "run_interrupted".to_owned(),
                    message: "run was interrupted before mutation dispatch".to_owned(),
                };
                break;
            }
            let (status, reason, navigated) = execute_v5_step(
                journey,
                run_root,
                driver,
                step,
                StepPhase::Setup,
                sequence,
                has_navigated,
                &fixture_token,
                report,
            );
            sequence += 1;
            has_navigated |= navigated;
            let verifies_auth = report
                .authentication
                .as_ref()
                .is_some_and(|auth| auth.verification_step == step.id);
            if status != RunOutcome::Passed {
                primary = if verifies_auth {
                    set_authentication_status(report, AuthenticationStatus::Blocked);
                    RunOutcome::Blocked
                } else if status == RunOutcome::Failed {
                    RunOutcome::Blocked
                } else {
                    status
                };
                primary_reason = if verifies_auth {
                    Reason {
                        code: "authentication_verification_failed".to_owned(),
                        message: "the disposable actor was not verified through the visible UI"
                            .to_owned(),
                    }
                } else if status == RunOutcome::Failed {
                    Reason {
                        code: "fixture_setup_failed".to_owned(),
                        message: "the disposable fixture setup precondition did not pass"
                            .to_owned(),
                    }
                } else {
                    reason
                };
                break;
            }
            if verifies_auth {
                set_authentication_status(report, AuthenticationStatus::Verified);
            }
        }
    }
    if let Some(fixture) = &mut report.fixture {
        fixture.setup_status = if pending_recovery.is_some() && primary == RunOutcome::Passed {
            FixtureSetupStatus::Blocked
        } else {
            match primary {
                RunOutcome::Passed => FixtureSetupStatus::Passed,
                RunOutcome::Failed => FixtureSetupStatus::Failed,
                RunOutcome::Blocked => FixtureSetupStatus::Blocked,
                RunOutcome::Error => FixtureSetupStatus::Error,
            }
        };
    }

    let mut recovery = None;
    if primary == RunOutcome::Passed {
        let record = pending_recovery.clone().unwrap_or_else(|| RecoveryRecord {
            schema_version: 1,
            journey_id: journey.meta.id.clone(),
            revision: journey.meta.revision,
            source_sha256: journey.source_sha256.clone(),
            target_origin: journey.origin.to_string(),
            run_id: run_id.to_owned(),
            run_directory: run_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned(),
            cleanup_step_ids: journey
                .cleanup_steps
                .iter()
                .map(|step| step.id.clone())
                .collect(),
            created_at_unix_ms: unix_ms(),
        });
        let recovery_result = if pending_recovery.is_some() {
            recovery_store.resume(record, run_root)
        } else {
            recovery_store.begin(record, run_root)
        };
        match recovery_result {
            Ok(active) => recovery = Some(active),
            Err(error) => {
                primary = RunOutcome::Blocked;
                primary_reason = Reason {
                    code: error.code().to_owned(),
                    message: error.to_string(),
                };
                if let Some(fixture) = &mut report.fixture {
                    fixture.recovery_required = matches!(
                        error,
                        crate::recovery::RecoveryError::PartialBegin
                            | crate::recovery::RecoveryError::Pending
                    );
                }
            }
        }
    }

    if primary == RunOutcome::Passed && pending_recovery.is_some() && recovery.is_some() {
        primary = RunOutcome::Blocked;
        primary_reason = Reason {
            code: "recovery_cleanup_required".to_owned(),
            message:
                "a prior interrupted mutation is being recovered through declared visible cleanup"
                    .to_owned(),
        };
    }

    if primary == RunOutcome::Passed && pending_recovery.is_none() {
        for step in &journey.steps {
            if interrupted.load(Ordering::Acquire) {
                primary = RunOutcome::Error;
                primary_reason = Reason {
                    code: "run_interrupted".to_owned(),
                    message: "run was interrupted; fixture cleanup is being attempted".to_owned(),
                };
                break;
            }
            let (status, reason, navigated) = execute_v5_step(
                journey,
                run_root,
                driver,
                step,
                StepPhase::Journey,
                sequence,
                has_navigated,
                &fixture_token,
                report,
            );
            sequence += 1;
            has_navigated |= navigated;
            if step.action.is_mutating()
                && report.steps.last().is_some_and(|report_step| {
                    !matches!(
                        report_step.observation.action_state,
                        None | Some(ActionState::NotAttempted)
                    )
                })
                && let Some(fixture) = &mut report.fixture
            {
                fixture.mutation_attempted = true;
            }
            if status != RunOutcome::Passed {
                primary = status;
                primary_reason = reason;
                break;
            }
        }
    }
    report.execution_outcome = primary;
    report.execution_reason = primary_reason.clone();
    report.outcome = primary;
    report.reason = primary_reason;

    if recovery.is_some() {
        driver.begin_fixture_cleanup();
        let mut cleanup_passed = true;
        let mut cleanup_unknown = false;
        for step in &journey.cleanup_steps {
            let (status, reason, navigated) = execute_v5_step(
                journey,
                run_root,
                driver,
                step,
                StepPhase::FixtureCleanup,
                sequence,
                has_navigated,
                &fixture_token,
                report,
            );
            sequence += 1;
            has_navigated |= navigated;
            if status != RunOutcome::Passed {
                cleanup_passed = false;
                cleanup_unknown = report.steps.last().is_some_and(|step| {
                    step.observation.action_state == Some(ActionState::EffectUnknown)
                });
                override_with_message(report, "fixture_cleanup_failed", reason.message);
                break;
            }
        }
        if cleanup_passed {
            match recovery
                .take()
                .expect("recovery handle exists")
                .complete_verified()
            {
                Ok(()) => {
                    if let Some(fixture) = &mut report.fixture {
                        fixture.cleanup_status = FixtureCleanupStatus::Passed;
                        fixture.recovery_required = false;
                    }
                    if pending_recovery.is_some() {
                        report.execution_outcome = RunOutcome::Blocked;
                        report.execution_reason = Reason {
                            code: "recovery_completed".to_owned(),
                            message: "the prior mutation's declared visible cleanup was verified; run the journey again to start a new mutation"
                                .to_owned(),
                        };
                        report.outcome = report.execution_outcome;
                        report.reason = report.execution_reason.clone();
                    }
                }
                Err(error) => {
                    if let Some(fixture) = &mut report.fixture {
                        fixture.cleanup_status = FixtureCleanupStatus::Failed;
                        fixture.recovery_required = true;
                    }
                    override_with_message(report, "recovery_complete_failed", error.to_string());
                }
            }
        } else if let Some(fixture) = &mut report.fixture {
            fixture.cleanup_status = if cleanup_unknown {
                FixtureCleanupStatus::EffectUnknown
            } else {
                FixtureCleanupStatus::Failed
            };
            fixture.recovery_required = true;
        }
    }

    finish_mutating_session(
        journey,
        run_root,
        driver,
        prepared,
        trace_started,
        report.execution_outcome,
        report.execution_reason.clone(),
        report,
    );
}

#[allow(clippy::too_many_arguments)]
fn execute_v5_step(
    journey: &ValidatedJourney,
    run_root: &Path,
    driver: &mut dyn BrowserDriver,
    step: &crate::journey::ValidatedStep,
    phase: StepPhase,
    index: usize,
    has_navigated: bool,
    fixture_token: &str,
    report: &mut RunReport,
) -> (RunOutcome, Reason, bool) {
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
        Some(fixture_token),
        &mut observation,
        &mut report.artifacts,
    );
    let navigated = matches!(step.action, ValidatedAction::Navigate { .. }) && result.is_ok();
    let (status, reason) = match result {
        Ok(StepResult::Passed) => (
            RunOutcome::Passed,
            Reason {
                code: "step_passed".to_owned(),
                message: "declared step passed".to_owned(),
            },
        ),
        Ok(StepResult::Failed(message)) => (
            RunOutcome::Failed,
            Reason {
                code: "checkpoint_failed".to_owned(),
                message,
            },
        ),
        Ok(StepResult::Blocked(message)) => (
            RunOutcome::Blocked,
            Reason {
                code: "origin_not_authorized".to_owned(),
                message,
            },
        ),
        Err(error) => {
            let mut outcome = RunOutcome::Passed;
            let mut reason = Reason {
                code: "step_error".to_owned(),
                message: "step failed".to_owned(),
            };
            set_driver_error(&mut outcome, &mut reason, error);
            (outcome, reason)
        }
    };
    report.steps.push(StepReport {
        sequence: index as u32 + 1,
        id: step.id.clone(),
        title: step.title.clone(),
        phase: Some(phase),
        effect: Some(step.effect),
        kind,
        status,
        started_at_unix_ms: started,
        duration_ms: elapsed_ms(timer),
        observation,
    });
    (status, reason, navigated)
}

#[allow(clippy::too_many_arguments)]
fn finish_mutating_session(
    journey: &ValidatedJourney,
    run_root: &Path,
    driver: &mut dyn BrowserDriver,
    prepared: bool,
    trace_started: bool,
    primary: RunOutcome,
    primary_reason: Reason,
    report: &mut RunReport,
) {
    if report.execution_reason.code == "run_incomplete" {
        report.execution_outcome = primary;
        report.execution_reason = primary_reason.clone();
        report.outcome = primary;
        report.reason = primary_reason;
    }
    if prepared && trace_started && journey.evidence.diagnostics {
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
                    Err(error) => {
                        override_with_message(report, "trace_artifact_invalid", error.to_string())
                    }
                }
            }
            Err(error) => override_with_evidence_error(report, "trace_finalization_failed", error),
        }
    }
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

fn mutation_interrupt_flag() -> Result<Arc<AtomicBool>, String> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    if let Some(flag) = FLAG.get() {
        return Ok(Arc::clone(flag));
    }
    let flag = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&flag);
    ctrlc::set_handler(move || handler_flag.store(true, Ordering::Release))
        .map_err(|error| error.to_string())?;
    let _ = FLAG.set(Arc::clone(&flag));
    Ok(flag)
}

fn fixture_token(run_id: &str) -> String {
    let suffix = run_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .take(40)
        .collect::<String>();
    format!(
        "crawlson-fixture-{}",
        if suffix.is_empty() { "run" } else { &suffix }
    )
}

fn execute(
    journey: &ValidatedJourney,
    run_root: &Path,
    driver: &mut dyn BrowserDriver,
    authentication_state: Option<ValidatedState>,
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
        message: if journey.schema_version >= 3 {
            "all declared steps, authorized actions, and required evidence completed".to_owned()
        } else {
            "all read-only steps and required evidence completed".to_owned()
        },
    };

    match driver.prepare() {
        Ok(()) => prepared = true,
        Err(error) => set_driver_error(&mut primary, &mut primary_reason, error),
    }
    if primary == RunOutcome::Passed
        && let Some(authentication_state) = authentication_state
    {
        match authentication_state.stage() {
            Ok(staged) => {
                let load_result = driver.load_authentication(staged.path());
                let cleanup_result = staged.close();
                if load_result.is_err() || cleanup_result.is_err() {
                    set_authentication_status(report, AuthenticationStatus::LoadFailed);
                    primary = RunOutcome::Error;
                    primary_reason = Reason {
                        code: "authentication_state_load_failed".to_owned(),
                        message:
                            "agent-browser could not safely load the supplied authentication state"
                                .to_owned(),
                    };
                }
            }
            Err(_) => {
                set_authentication_status(report, AuthenticationStatus::LoadFailed);
                primary = RunOutcome::Error;
                primary_reason = Reason {
                    code: "authentication_state_load_failed".to_owned(),
                    message:
                        "agent-browser could not safely load the supplied authentication state"
                            .to_owned(),
                };
            }
        }
        drop(authentication_state);
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
                None,
                &mut observation,
                &mut report.artifacts,
            );
            if matches!(step.action, ValidatedAction::Navigate { .. }) && result.is_ok() {
                has_navigated = true;
            }
            let mut stop = false;
            let is_authentication_verification = report
                .authentication
                .as_ref()
                .is_some_and(|authentication| authentication.verification_step == step.id);
            let status = match result {
                Ok(StepResult::Passed) => {
                    if is_authentication_verification {
                        set_authentication_status(report, AuthenticationStatus::Verified);
                    }
                    RunOutcome::Passed
                }
                Ok(StepResult::Failed(message)) => {
                    if is_authentication_verification {
                        primary = RunOutcome::Blocked;
                        primary_reason = Reason {
                            code: "authentication_verification_failed".to_owned(),
                            message:
                                "the visible authentication verification checkpoint did not pass"
                                    .to_owned(),
                        };
                        stop = true;
                    } else if primary == RunOutcome::Passed {
                        primary = RunOutcome::Failed;
                        primary_reason = Reason {
                            code: "checkpoint_failed".to_owned(),
                            message,
                        };
                    }
                    if kind == "follow_link" {
                        stop = true;
                    }
                    if is_authentication_verification {
                        RunOutcome::Blocked
                    } else {
                        RunOutcome::Failed
                    }
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
                phase: None,
                effect: None,
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

    if opened
        && prepared
        && trace_started
        && journey.evidence.diagnostics
        && primary != RunOutcome::Blocked
    {
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
    fixture_token: Option<&str>,
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
        ValidatedAction::FollowLink {
            selector,
            expected_url,
            alt_text,
        } => {
            observation.action_state = Some(ActionState::NotAttempted);
            observation.action_grant_sha256 = Some(action_step_binding(journey, step_id));
            observation.expected_url = Some(safe_url(&journey.origin, expected_url));

            let before = driver.current_url()?;
            observation.before_url = Some(safe_url(&journey.origin, &before));
            if !journey.origin.contains(&before) {
                return Ok(StepResult::Blocked(format!(
                    "link preflight started outside authorized origin {}",
                    journey.origin
                )));
            }

            // Force every inspection and dispatch through an anchor-only CSS
            // selector. The exact href is added after it is read so a button,
            // custom element, or link whose href changes cannot satisfy the
            // click selector used for this action.
            let anchor_selector = anchor_selector(selector);
            let visible = driver.visible(&anchor_selector)?;
            observation.visible = Some(visible);
            if !visible {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared target was not a visible link".to_owned(),
                ));
            }
            let enabled = driver.enabled(&anchor_selector)?;
            observation.enabled = Some(enabled);
            if !enabled {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared link target was not enabled".to_owned(),
                ));
            }

            let href = driver.attribute(&anchor_selector, "href")?;
            let Some(href) = href else {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared link target did not expose an href".to_owned(),
                ));
            };
            if href.len() > 8_192 || href.chars().any(char::is_control) {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared link href was not a bounded single-line URL".to_owned(),
                ));
            }
            let resolved = match before.join(&href) {
                Ok(url) => url,
                Err(_) => {
                    observation.matched = Some(false);
                    return Ok(StepResult::Failed(
                        "declared link href was not a valid URL".to_owned(),
                    ));
                }
            };
            observation.target_href = Some(safe_url(&journey.origin, &resolved));
            let dispatch_selector = exact_anchor_selector(selector, &href);

            let stem = format!("{:03}-{}", index + 1, step_id);
            let raw = run_root.join("evidence").join(format!("{stem}.raw.png"));
            let capture = driver.capture(&dispatch_selector, &raw)?;

            if !resolved.username().is_empty()
                || resolved.password().is_some()
                || resolved.query().is_some()
                || resolved.fragment().is_some()
                || !journey.origin.contains(&resolved)
            {
                finalize_focused_capture(
                    run_root,
                    step_id,
                    index,
                    alt_text,
                    capture,
                    None,
                    observation,
                    artifacts,
                )?;
                return Ok(StepResult::Blocked(
                    "declared link href was outside the authorized exact-origin contract"
                        .to_owned(),
                ));
            }
            if resolved != *expected_url {
                finalize_focused_capture(
                    run_root,
                    step_id,
                    index,
                    alt_text,
                    capture,
                    None,
                    observation,
                    artifacts,
                )?;
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared link href did not match the expected destination".to_owned(),
                ));
            }

            let screenshot_sequence = capture.screenshot_command_sequence;
            let prior_records = driver.records().len();
            let click = driver.click(&dispatch_selector);
            let records = driver.records();
            let click_sequence = records
                .last()
                .filter(|record| records.len() == prior_records + 1 && record.capability == "click")
                .map(|record| record.sequence)
                .filter(|sequence| *sequence == screenshot_sequence.saturating_add(1));
            let Some(click_sequence) = click_sequence else {
                observation.action_state = Some(ActionState::EffectUnknown);
                let _ = finalize_focused_capture(
                    run_root,
                    step_id,
                    index,
                    alt_text,
                    capture,
                    None,
                    observation,
                    artifacts,
                );
                return Err(DriverError::ActionEffectUnknown(
                    "click dispatch did not produce adjacent command provenance".to_owned(),
                ));
            };
            observation.action_command_sequence = Some(click_sequence);
            if let Err(error) = finalize_focused_capture(
                run_root,
                step_id,
                index,
                alt_text,
                capture,
                Some(click_sequence),
                observation,
                artifacts,
            ) {
                observation.action_state = Some(ActionState::EffectUnknown);
                return Err(DriverError::ActionEffectUnknown(error.to_string()));
            }
            match click {
                Ok(()) => observation.action_state = Some(ActionState::DriverAcknowledged),
                Err(error @ DriverError::ConfirmationRequired { .. }) => return Err(error),
                Err(error) => {
                    observation.action_state = Some(ActionState::EffectUnknown);
                    return Err(DriverError::ActionEffectUnknown(error.to_string()));
                }
            }

            let after = match driver.current_url() {
                Ok(url) => url,
                Err(error) => {
                    observation.action_state = Some(ActionState::EffectUnknown);
                    return Err(DriverError::ActionEffectUnknown(error.to_string()));
                }
            };
            observation.observed_url = Some(safe_url(&journey.origin, &after));
            if !journey.origin.contains(&after) {
                StepResult::Blocked(format!(
                    "link action reached outside authorized origin {}",
                    journey.origin
                ))
            } else if after == *expected_url {
                observation.matched = Some(true);
                observation.action_state = Some(ActionState::EffectVerified);
                StepResult::Passed
            } else {
                observation.matched = Some(false);
                StepResult::Failed("link action did not reach the declared destination".to_owned())
            }
        }
        ValidatedAction::FillText {
            selector,
            value,
            alt_text,
        } => {
            observation.action_state = Some(ActionState::NotAttempted);
            observation.action_grant_sha256 = Some(mutation_step_binding(journey, step_id));
            observation.guard_command_sequence = Some(driver.verify_exact_origin_guard()?);
            let value = match (value, fixture_token) {
                (MutationValue::FixtureToken, Some(value)) => value,
                _ => {
                    return Err(DriverError::Protocol {
                        capability: "fill".to_owned(),
                        message: "generated fixture token was unavailable".to_owned(),
                    });
                }
            };
            let dispatch_selector = exact_text_input_selector(selector);
            if driver.count(&dispatch_selector)? != 1 {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared fixture input did not uniquely match an ordinary text field"
                        .to_owned(),
                ));
            }
            let visible = driver.visible(&dispatch_selector)?;
            observation.visible = Some(visible);
            if !visible {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared fixture input was not visible".to_owned(),
                ));
            }
            let enabled = driver.enabled(&dispatch_selector)?;
            observation.enabled = Some(enabled);
            if !enabled {
                observation.matched = Some(false);
                return Ok(StepResult::Failed(
                    "declared fixture input was not enabled".to_owned(),
                ));
            }
            let before = driver.current_url()?;
            observation.before_url = Some(safe_url(&journey.origin, &before));
            let stem = format!("{:03}-{}", index + 1, step_id);
            let raw = run_root.join("evidence").join(format!("{stem}.raw.png"));
            let capture = driver.capture(&dispatch_selector, &raw)?;
            let screenshot_sequence = capture.screenshot_command_sequence;
            let prior_records = driver.records().len();
            let fill_result = driver.fill(&dispatch_selector, value);
            let records = driver.records();
            let fill_sequence = records
                .last()
                .filter(|record| records.len() == prior_records + 1 && record.capability == "fill")
                .map(|record| record.sequence)
                .filter(|sequence| *sequence == screenshot_sequence.saturating_add(1));
            let Some(fill_sequence) = fill_sequence else {
                observation.action_state = Some(ActionState::EffectUnknown);
                let _ = finalize_focused_capture(
                    run_root,
                    step_id,
                    index,
                    alt_text,
                    capture,
                    None,
                    observation,
                    artifacts,
                );
                return Err(DriverError::ActionEffectUnknown(
                    "fill dispatch did not produce adjacent command provenance".to_owned(),
                ));
            };
            observation.action_command_sequence = Some(fill_sequence);
            if let Err(error) = finalize_focused_capture(
                run_root,
                step_id,
                index,
                alt_text,
                capture,
                Some(fill_sequence),
                observation,
                artifacts,
            ) {
                observation.action_state = Some(ActionState::EffectUnknown);
                return Err(DriverError::ActionEffectUnknown(error.to_string()));
            }
            if let Err(error) = fill_result {
                observation.action_state = Some(ActionState::EffectUnknown);
                return Err(DriverError::ActionEffectUnknown(error.to_string()));
            }
            observation.action_state = Some(ActionState::DriverAcknowledged);
            let actual = driver.value(&dispatch_selector).map_err(|error| {
                observation.action_state = Some(ActionState::EffectUnknown);
                DriverError::ActionEffectUnknown(error.to_string())
            })?;
            observation.observed_text_sha256 = Some(hex_digest(actual.as_bytes()));
            let after = driver.current_url().map_err(|error| {
                observation.action_state = Some(ActionState::EffectUnknown);
                DriverError::ActionEffectUnknown(error.to_string())
            })?;
            observation.observed_url = Some(safe_url(&journey.origin, &after));
            let matched = actual == value && after == before;
            observation.matched = Some(matched);
            if matched {
                observation.action_state = Some(ActionState::EffectVerified);
                StepResult::Passed
            } else {
                observation.action_state = Some(ActionState::EffectUnverified);
                StepResult::Failed(
                    "fixture input value or page location did not match after fill".to_owned(),
                )
            }
        }
        ValidatedAction::ClickButton {
            selector,
            form_selector,
            action_url,
            expected_url,
            verify_selector,
            expected_text,
            alt_text,
        } => execute_button_mutation(
            journey,
            run_root,
            driver,
            step_id,
            index,
            selector,
            form_selector,
            action_url,
            expected_url,
            verify_selector,
            expected_text,
            alt_text,
            observation,
            artifacts,
        )?,
        ValidatedAction::CheckAbsent {
            selector,
            expected_text,
        } => {
            let visible = driver.visible(selector)?;
            observation.visible = Some(visible);
            if !visible {
                observation.matched = Some(false);
                StepResult::Failed("disposable fixture absence marker was not visible".to_owned())
            } else {
                let actual = driver.text(selector)?;
                observation.observed_text_sha256 = Some(hex_digest(actual.as_bytes()));
                let matched = actual == *expected_text;
                observation.matched = Some(matched);
                if matched {
                    StepResult::Passed
                } else {
                    StepResult::Failed(
                        "disposable fixture was not in the declared absent state".to_owned(),
                    )
                }
            }
        }
        ValidatedAction::EnsureAbsent {
            status_selector,
            expected_text,
            button_selector,
            form_selector,
            action_url,
            expected_url,
            alt_text,
        } => {
            observation.action_state = Some(ActionState::NotAttempted);
            observation.action_grant_sha256 = Some(mutation_step_binding(journey, step_id));
            let status_count = driver.count(status_selector)?;
            if status_count > 1 {
                observation.guard_command_sequence = Some(driver.verify_exact_origin_guard()?);
                observation.matched = Some(false);
                StepResult::Failed("fixture cleanup absence marker was ambiguous".to_owned())
            } else if status_count == 1 && driver.visible(status_selector)? {
                observation.guard_command_sequence = Some(driver.verify_exact_origin_guard()?);
                let actual = driver.text(status_selector)?;
                observation.observed_text_sha256 = Some(hex_digest(actual.as_bytes()));
                if actual == *expected_text {
                    observation.visible = Some(true);
                    observation.matched = Some(true);
                    observation.action_state = Some(ActionState::EffectVerified);
                    StepResult::Passed
                } else {
                    observation.matched = Some(false);
                    StepResult::Failed("fixture cleanup absence marker did not match".to_owned())
                }
            } else {
                execute_button_mutation(
                    journey,
                    run_root,
                    driver,
                    step_id,
                    index,
                    button_selector,
                    form_selector,
                    action_url,
                    expected_url,
                    status_selector,
                    expected_text,
                    alt_text,
                    observation,
                    artifacts,
                )?
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
            let capture = driver.capture(selector, &raw)?;
            finalize_focused_capture(
                run_root,
                step_id,
                index,
                alt_text,
                capture,
                None,
                observation,
                artifacts,
            )?;
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

#[allow(clippy::too_many_arguments)]
fn execute_button_mutation(
    journey: &ValidatedJourney,
    run_root: &Path,
    driver: &mut dyn BrowserDriver,
    step_id: &str,
    index: usize,
    selector: &str,
    form_selector: &str,
    action_url: &url::Url,
    expected_url: &url::Url,
    verify_selector: &str,
    expected_text: &str,
    alt_text: &str,
    observation: &mut StepObservation,
    artifacts: &mut Vec<ArtifactRecord>,
) -> Result<StepResult, DriverError> {
    observation.action_state = Some(ActionState::NotAttempted);
    observation.action_grant_sha256 = Some(mutation_step_binding(journey, step_id));
    observation.guard_command_sequence = Some(driver.verify_exact_origin_guard()?);
    observation.expected_url = Some(safe_url(&journey.origin, expected_url));
    let before = driver.current_url()?;
    observation.before_url = Some(safe_url(&journey.origin, &before));
    if !journey.origin.contains(&before) {
        return Ok(StepResult::Blocked(
            "mutation preflight started outside the authorized origin".to_owned(),
        ));
    }
    let form = exact_form_selector(form_selector);
    let button = exact_submit_selector(form_selector, selector);
    if driver.count(&form)? != 1 || driver.count(&button)? != 1 {
        observation.matched = Some(false);
        return Ok(StepResult::Failed(
            "declared mutation form and submit button did not each match exactly once".to_owned(),
        ));
    }
    let visible = driver.visible(&button)?;
    observation.visible = Some(visible);
    if !visible {
        observation.matched = Some(false);
        return Ok(StepResult::Failed(
            "declared mutation submit button was not visible".to_owned(),
        ));
    }
    let enabled = driver.enabled(&button)?;
    observation.enabled = Some(enabled);
    if !enabled {
        observation.matched = Some(false);
        return Ok(StepResult::Failed(
            "declared mutation submit button was not enabled".to_owned(),
        ));
    }
    let method = driver.attribute(&form, "method")?;
    let action = driver.attribute(&form, "action")?;
    let form_target = driver.attribute(&form, "target")?;
    let button_method = driver.attribute(&button, "formmethod")?;
    let button_action = driver.attribute(&button, "formaction")?;
    let button_target = driver.attribute(&button, "formtarget")?;
    let resolved_action = action.as_deref().and_then(|value| before.join(value).ok());
    let target_safe = |value: &Option<String>| {
        value
            .as_deref()
            .is_none_or(|value| value.is_empty() || value.eq_ignore_ascii_case("_self"))
    };
    if method.as_deref().map(str::to_ascii_lowercase).as_deref() != Some("post")
        || resolved_action.as_ref() != Some(action_url)
        || action_url.query().is_some()
        || action_url.fragment().is_some()
        || !journey.origin.contains(action_url)
        || button_method
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || button_action
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || !target_safe(&form_target)
        || !target_safe(&button_target)
    {
        observation.matched = Some(false);
        return Ok(StepResult::Failed(
            "mutation form did not satisfy the exact POST same-origin preflight".to_owned(),
        ));
    }

    let stem = format!("{:03}-{}", index + 1, step_id);
    let raw = run_root.join("evidence").join(format!("{stem}.raw.png"));
    let capture = driver.capture(&button, &raw)?;
    let screenshot_sequence = capture.screenshot_command_sequence;
    let prior_records = driver.records().len();
    let click = driver.click(&button);
    let records = driver.records();
    let click_sequence = records
        .last()
        .filter(|record| records.len() == prior_records + 1 && record.capability == "click")
        .map(|record| record.sequence)
        .filter(|sequence| *sequence == screenshot_sequence.saturating_add(1));
    let Some(click_sequence) = click_sequence else {
        observation.action_state = Some(ActionState::EffectUnknown);
        let _ = finalize_focused_capture(
            run_root,
            step_id,
            index,
            alt_text,
            capture,
            None,
            observation,
            artifacts,
        );
        return Err(DriverError::ActionEffectUnknown(
            "mutation click did not produce adjacent command provenance".to_owned(),
        ));
    };
    observation.action_command_sequence = Some(click_sequence);
    if let Err(error) = finalize_focused_capture(
        run_root,
        step_id,
        index,
        alt_text,
        capture,
        Some(click_sequence),
        observation,
        artifacts,
    ) {
        observation.action_state = Some(ActionState::EffectUnknown);
        return Err(DriverError::ActionEffectUnknown(error.to_string()));
    }
    if let Err(error) = click {
        observation.action_state = Some(ActionState::EffectUnknown);
        return Err(DriverError::ActionEffectUnknown(error.to_string()));
    }
    observation.action_state = Some(ActionState::DriverAcknowledged);
    let after = driver.current_url().map_err(|error| {
        observation.action_state = Some(ActionState::EffectUnknown);
        DriverError::ActionEffectUnknown(error.to_string())
    })?;
    observation.observed_url = Some(safe_url(&journey.origin, &after));
    if !journey.origin.contains(&after) {
        observation.action_state = Some(ActionState::EffectUnknown);
        return Err(DriverError::ActionEffectUnknown(
            "mutation ended outside the exact authorized origin".to_owned(),
        ));
    }
    if after != *expected_url {
        observation.matched = Some(false);
        observation.action_state = Some(ActionState::EffectUnverified);
        return Ok(StepResult::Failed(
            "mutation did not reach the declared postcondition URL".to_owned(),
        ));
    }
    let post_visible = driver.visible(verify_selector).map_err(|error| {
        observation.action_state = Some(ActionState::EffectUnknown);
        DriverError::ActionEffectUnknown(error.to_string())
    })?;
    if !post_visible {
        observation.visible = Some(false);
        observation.matched = Some(false);
        observation.action_state = Some(ActionState::EffectUnverified);
        return Ok(StepResult::Failed(
            "mutation postcondition marker was not visible".to_owned(),
        ));
    }
    let actual = driver.text(verify_selector).map_err(|error| {
        observation.action_state = Some(ActionState::EffectUnknown);
        DriverError::ActionEffectUnknown(error.to_string())
    })?;
    observation.visible = Some(true);
    observation.observed_text_sha256 = Some(hex_digest(actual.as_bytes()));
    let matched = actual == expected_text;
    observation.matched = Some(matched);
    if matched {
        observation.action_state = Some(ActionState::EffectVerified);
        Ok(StepResult::Passed)
    } else {
        observation.action_state = Some(ActionState::EffectUnverified);
        Ok(StepResult::Failed(
            "mutation visible postcondition did not match".to_owned(),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn finalize_focused_capture(
    run_root: &Path,
    step_id: &str,
    index: usize,
    alt_text: &str,
    capture: CaptureBundle,
    action_command_sequence: Option<u32>,
    observation: &mut StepObservation,
    artifacts: &mut Vec<ArtifactRecord>,
) -> Result<(), DriverError> {
    let stem = format!("{:03}-{}", index + 1, step_id);
    let focused = run_root
        .join("evidence")
        .join(format!("{stem}.focused.png"));
    let metadata = run_root
        .join("evidence")
        .join(format!("{stem}.focused.json"));
    observation.target_box_css = Some(capture.target);
    observation.viewport = Some(capture.viewport);
    observation.box_command_sequence = Some(capture.box_command_sequence);
    observation.screenshot_command_sequence = Some(capture.screenshot_command_sequence);
    let capture_token = action_command_sequence.map_or_else(
        || capture.capture_token.clone(),
        |sequence| format!("{}:{sequence}", capture.capture_token),
    );
    observation.capture_token = Some(capture_token.clone());
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
        capture_token: &capture_token,
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
    Ok(())
}

fn action_step_binding(journey: &ValidatedJourney, step_id: &str) -> String {
    let binding = format!(
        "crawlson-action-step-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nstep={}\ngrant={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        step_id,
        action_grant(journey, step_id)
    );
    hex_digest(binding.as_bytes())
}

fn mutation_step_binding(journey: &ValidatedJourney, step_id: &str) -> String {
    let grant = action_grant(journey, step_id);
    let binding = format!(
        "crawlson-mutation-step-v1\njourney={}\nrevision={}\nsource_sha256={}\norigin={}\nstep={}\ngrant={}\n",
        journey.meta.id,
        journey.meta.revision,
        journey.source_sha256,
        journey.origin,
        step_id,
        grant
    );
    hex_digest(binding.as_bytes())
}

fn selector_id(selector: &str) -> &str {
    selector
        .strip_prefix('#')
        .expect("validated mutation selector is a simple #id")
}

fn exact_text_input_selector(selector: &str) -> String {
    let id = selector_id(selector);
    format!("input#{id}:not([type=password]):not([type=file]):not([type=hidden]),textarea#{id}")
}

fn exact_form_selector(selector: &str) -> String {
    format!("form#{}", selector_id(selector))
}

fn exact_submit_selector(form_selector: &str, selector: &str) -> String {
    let form = selector_id(form_selector);
    let button = selector_id(selector);
    format!("form#{form} button#{button}[type=submit],form#{form} input#{button}[type=submit]")
}

fn anchor_selector(selector: &str) -> String {
    format!(":is({selector}):is(a[href])")
}

fn exact_anchor_selector(selector: &str, href: &str) -> String {
    let mut escaped = String::with_capacity(href.len());
    for character in href.chars() {
        if matches!(character, '\\' | '"') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    format!(":is({selector}):is(a[href=\"{escaped}\"])")
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
        action_authorization: None,
        mutation_authorization: None,
        authentication: None,
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
        fixture: None,
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
        DriverError::ConfirmationRequired { .. } => "driver_confirmation_required",
        DriverError::ActionEffectUnknown(_) => "action_effect_unknown",
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
        DriverError::ConfirmationRequired { capability } => format!(
            "agent-browser command '{capability}' required confirmation and was not executed"
        ),
        DriverError::ActionEffectUnknown(_) => {
            "browser action effect is unknown after dispatch".to_owned()
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
        ValidatedAction::FollowLink { .. } => "follow_link",
        ValidatedAction::Capture { .. } => "capture",
        ValidatedAction::FillText { .. } => "fill_text",
        ValidatedAction::ClickButton { .. } => "click_button",
        ValidatedAction::CheckAbsent { .. } => "check_absent",
        ValidatedAction::EnsureAbsent { .. } => "ensure_absent",
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
    use crate::driver::{CaptureBundle, DiagnosticsSummary};
    use crate::focus::{CssBox, Viewport};
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
            schema_version: 1,
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
            fixture: None,
            evidence: EvidencePolicy {
                trace: true,
                diagnostics: true,
            },
            setup_steps: Vec::new(),
            steps: vec![
                ValidatedStep {
                    id: "open".to_owned(),
                    title: "Open".to_owned(),
                    guide_instruction: None,
                    evidence_for: Vec::new(),
                    effect: StepEffect::ReadOnly,
                    action: ValidatedAction::Navigate {
                        url: Url::parse("http://127.0.0.1:4173/").unwrap(),
                    },
                },
                ValidatedStep {
                    id: "check".to_owned(),
                    title: "Check".to_owned(),
                    guide_instruction: None,
                    evidence_for: Vec::new(),
                    effect: StepEffect::ReadOnly,
                    action: ValidatedAction::CheckText {
                        selector: "h1".to_owned(),
                        expected: expected.to_owned(),
                        comparison: TextComparison::Exact,
                    },
                },
            ],
            cleanup_steps: Vec::new(),
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
            None,
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
            None,
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
            None,
            &mut report,
        );
        assert_eq!(report.execution_outcome, RunOutcome::Blocked);
        assert_eq!(report.reason.code, "origin_not_authorized");
        assert_eq!(report.steps.len(), 1);
    }

    #[test]
    fn unknown_click_provenance_still_preserves_pre_action_evidence() {
        struct MissingClickRecordDriver {
            url: Url,
        }

        impl BrowserDriver for MissingClickRecordDriver {
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
                Ok(String::new())
            }
            fn visible(&mut self, _selector: &str) -> Result<bool, DriverError> {
                Ok(true)
            }
            fn enabled(&mut self, _selector: &str) -> Result<bool, DriverError> {
                Ok(true)
            }
            fn attribute(
                &mut self,
                _selector: &str,
                _name: &str,
            ) -> Result<Option<String>, DriverError> {
                Ok(Some("/complete".to_owned()))
            }
            fn click(&mut self, _selector: &str) -> Result<(), DriverError> {
                self.url = Url::parse("http://127.0.0.1:4173/complete").unwrap();
                Ok(())
            }
            fn capture(
                &mut self,
                _selector: &str,
                path: &Path,
            ) -> Result<CaptureBundle, DriverError> {
                let file = fs::File::create(path).unwrap();
                let mut encoder = png::Encoder::new(file, 1280, 720);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&vec![255; 1280 * 720 * 4]).unwrap();
                Ok(CaptureBundle {
                    raw_path: path.to_path_buf(),
                    target: CssBox {
                        x: 100.0,
                        y: 100.0,
                        width: 200.0,
                        height: 50.0,
                    },
                    viewport: Viewport {
                        width_css: 1280.0,
                        height_css: 720.0,
                        device_scale_factor: 1.0,
                        scroll_x_css: Some(0.0),
                        scroll_y_css: Some(0.0),
                    },
                    capture_token: "fixture:1:2".to_owned(),
                    box_command_sequence: 1,
                    screenshot_command_sequence: 2,
                })
            }
            fn diagnostics(&mut self) -> Result<DiagnosticsSummary, DriverError> {
                unreachable!()
            }
            fn stop_trace(&mut self, _path: &Path) -> Result<PathBuf, DriverError> {
                unreachable!()
            }
            fn close(&mut self) -> Result<(), DriverError> {
                Ok(())
            }
            fn records(&self) -> Vec<DriverCommandRecord> {
                Vec::new()
            }
        }

        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("evidence")).unwrap();
        let mut journey = validated("Hello");
        journey.schema_version = 3;
        let action = ValidatedAction::FollowLink {
            selector: "#continue".to_owned(),
            expected_url: Url::parse("http://127.0.0.1:4173/complete").unwrap(),
            alt_text: "Continue highlighted in red".to_owned(),
        };
        let mut driver = MissingClickRecordDriver {
            url: Url::parse("http://127.0.0.1:4173/").unwrap(),
        };
        let mut observation = StepObservation::default();
        let mut artifacts = Vec::new();
        let error = match execute_step(
            &journey,
            directory.path(),
            &mut driver,
            "continue",
            0,
            &action,
            true,
            None,
            &mut observation,
            &mut artifacts,
        ) {
            Err(error) => error,
            Ok(_) => panic!("missing click provenance must not produce a step result"),
        };

        assert!(matches!(error, DriverError::ActionEffectUnknown(_)));
        assert_eq!(observation.action_state, Some(ActionState::EffectUnknown));
        assert!(observation.action_command_sequence.is_none());
        assert_eq!(
            artifacts
                .iter()
                .map(|artifact| artifact.kind)
                .collect::<Vec<_>>(),
            vec!["raw_screenshot", "focused_screenshot", "focus_metadata"]
        );
    }
}
