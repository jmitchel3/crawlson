use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

const MAX_JOURNEY_BYTES: u64 = 1_048_576;
const MAX_STEPS: usize = 256;
const MAX_SELECTOR_BYTES: usize = 4_096;
const MAX_EXPECTED_BYTES: usize = 65_536;
const MAX_ALT_TEXT_BYTES: usize = 4_096;
const MAX_GUIDE_INSTRUCTION_BYTES: usize = 16_384;
const MAX_FIXTURE_LIFETIME_SECONDS: u64 = 3_600;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyDocument {
    pub schema_version: u8,
    pub journey: JourneyMeta,
    pub target: TargetSpec,
    #[serde(default)]
    pub authentication: Option<AuthRequirement>,
    #[serde(default)]
    pub fixture: Option<FixtureSpec>,
    pub evidence: EvidencePolicy,
    #[serde(default)]
    pub setup_steps: Vec<StepSpec>,
    pub steps: Vec<StepSpec>,
    #[serde(default)]
    pub cleanup_steps: Vec<StepSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyMeta {
    pub id: String,
    pub revision: u32,
    pub title: String,
    pub purpose: String,
    pub expected_outcome: String,
    pub mode: JourneyMode,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JourneyMode {
    ReadOnly,
    Mutating,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    pub origin: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRequirement {
    pub provider: String,
    pub role: String,
    #[serde(default)]
    pub verification_step: Option<String>,
    #[serde(default)]
    pub disposable_actor: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureSpec {
    pub kind: FixtureKind,
    pub maximum_lifetime_seconds: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FixtureKind {
    SelfExpiringUi,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePolicy {
    pub trace: bool,
    pub diagnostics: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSpec {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub guide_instruction: Option<String>,
    #[serde(default)]
    pub evidence_for: Vec<String>,
    #[serde(default)]
    pub effect: Option<StepEffect>,
    pub action: StepAction,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepEffect {
    ReadOnly,
    Mutating,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepAction {
    Navigate {
        path: String,
    },
    CheckUrl {
        path: String,
    },
    CheckText {
        selector: String,
        expected: String,
        #[serde(default)]
        comparison: TextComparison,
    },
    Capture {
        selector: String,
        alt_text: String,
    },
    FollowLink {
        selector: String,
        expected_path: String,
        alt_text: String,
    },
    FillText {
        selector: String,
        value: String,
        alt_text: String,
    },
    ClickButton {
        selector: String,
        form_selector: String,
        action_path: String,
        expected_path: String,
        verify_selector: String,
        expected_text: String,
        alt_text: String,
    },
    CheckAbsent {
        selector: String,
        expected_text: String,
    },
    EnsureAbsent {
        status_selector: String,
        expected_text: String,
        button_selector: String,
        form_selector: String,
        action_path: String,
        expected_path: String,
        alt_text: String,
    },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextComparison {
    #[default]
    Exact,
    Contains,
}

#[derive(Debug, Clone)]
pub struct LoadedJourney {
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub document: JourneyDocument,
}

#[derive(Debug, Clone)]
pub struct ValidatedJourney {
    pub schema_version: u8,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub meta: JourneyMeta,
    pub origin: Origin,
    pub authentication: Option<AuthRequirement>,
    pub fixture: Option<FixtureSpec>,
    pub evidence: EvidencePolicy,
    pub setup_steps: Vec<ValidatedStep>,
    pub steps: Vec<ValidatedStep>,
    pub cleanup_steps: Vec<ValidatedStep>,
}

#[derive(Debug, Clone)]
pub struct ValidatedStep {
    pub id: String,
    pub title: String,
    pub guide_instruction: Option<String>,
    pub evidence_for: Vec<String>,
    pub effect: StepEffect,
    pub action: ValidatedAction,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepPhase {
    Setup,
    Journey,
    FixtureCleanup,
}

#[derive(Debug, Clone)]
pub enum ValidatedAction {
    Navigate {
        url: Url,
    },
    CheckUrl {
        url: Url,
    },
    CheckText {
        selector: String,
        expected: String,
        comparison: TextComparison,
    },
    Capture {
        selector: String,
        alt_text: String,
    },
    FollowLink {
        selector: String,
        expected_url: Url,
        alt_text: String,
    },
    FillText {
        selector: String,
        value: MutationValue,
        alt_text: String,
    },
    ClickButton {
        selector: String,
        form_selector: String,
        action_url: Url,
        expected_url: Url,
        verify_selector: String,
        expected_text: String,
        alt_text: String,
    },
    CheckAbsent {
        selector: String,
        expected_text: String,
    },
    EnsureAbsent {
        status_selector: String,
        expected_text: String,
        button_selector: String,
        form_selector: String,
        action_url: Url,
        expected_url: Url,
        alt_text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationValue {
    FixtureToken,
}

impl ValidatedAction {
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::FillText { .. } | Self::ClickButton { .. } | Self::EnsureAbsent { .. }
        )
    }
}

impl ValidatedJourney {
    pub fn phase_steps(&self) -> impl Iterator<Item = (StepPhase, &ValidatedStep)> {
        self.setup_steps
            .iter()
            .map(|step| (StepPhase::Setup, step))
            .chain(self.steps.iter().map(|step| (StepPhase::Journey, step)))
            .chain(
                self.cleanup_steps
                    .iter()
                    .map(|step| (StepPhase::FixtureCleanup, step)),
            )
    }

    pub fn mutating_steps(&self) -> impl Iterator<Item = &ValidatedStep> {
        self.phase_steps()
            .map(|(_, step)| step)
            .filter(|step| step.action.is_mutating())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Origin {
    pub scheme: String,
    pub host: String,
    pub effective_port: u16,
}

impl Origin {
    pub fn parse(value: &str) -> Result<Self, JourneyError> {
        let url = Url::parse(value)
            .map_err(|error| JourneyError::Validation(format!("invalid target origin: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(JourneyError::Validation(
                "target origin must use http or https".to_owned(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(JourneyError::Validation(
                "target origin must not contain credentials".to_owned(),
            ));
        }
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(JourneyError::Validation(
                "target origin must not contain a path, query, or fragment".to_owned(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| JourneyError::Validation("target origin requires a host".to_owned()))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        if host.ends_with('.') {
            return Err(JourneyError::Validation(
                "target origin host must not end with a dot".to_owned(),
            ));
        }
        let effective_port = url.port_or_known_default().ok_or_else(|| {
            JourneyError::Validation("target origin requires an effective port".to_owned())
        })?;
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host,
            effective_port,
        })
    }

    pub fn from_url(url: &Url) -> Result<Self, JourneyError> {
        if !url.username().is_empty() || url.password().is_some() {
            return Err(JourneyError::Validation(
                "URL must not contain credentials".to_owned(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| JourneyError::Validation("URL requires a host".to_owned()))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let effective_port = url
            .port_or_known_default()
            .ok_or_else(|| JourneyError::Validation("URL requires an effective port".to_owned()))?;
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host,
            effective_port,
        })
    }

    pub fn contains(&self, url: &Url) -> bool {
        Self::from_url(url).is_ok_and(|candidate| candidate == *self)
    }

    pub fn base_url(&self) -> Url {
        Url::parse(&format!("{self}/")).expect("validated origin formats as a URL")
    }

    pub fn is_literal_loopback(&self) -> bool {
        self.host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        write!(
            formatter,
            "{}://{}:{}",
            self.scheme, host, self.effective_port
        )
    }
}

#[derive(Debug, Error)]
pub enum JourneyError {
    #[error("could not read journey: {0}")]
    Read(String),
    #[error("journey is invalid: {0}")]
    Parse(String),
    #[error("journey validation failed: {0}")]
    Validation(String),
}

pub fn load(path: &Path) -> Result<LoadedJourney, JourneyError> {
    let file = fs::File::open(path).map_err(|error| JourneyError::Read(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| JourneyError::Read(error.to_string()))?;
    if !metadata.is_file() {
        return Err(JourneyError::Read("path is not a regular file".to_owned()));
    }
    if metadata.len() > MAX_JOURNEY_BYTES {
        return Err(JourneyError::Read(format!(
            "file exceeds {MAX_JOURNEY_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_JOURNEY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| JourneyError::Read(error.to_string()))?;
    if bytes.len() as u64 > MAX_JOURNEY_BYTES {
        return Err(JourneyError::Read(format!(
            "file exceeds {MAX_JOURNEY_BYTES} bytes"
        )));
    }
    let document = toml::from_slice(&bytes)
        .map_err(|error: toml::de::Error| JourneyError::Parse(error.message().to_owned()))?;
    Ok(LoadedJourney {
        source_path: path.to_path_buf(),
        source_sha256: hex_digest(&bytes),
        document,
    })
}

pub fn validate(loaded: LoadedJourney) -> Result<ValidatedJourney, JourneyError> {
    let document = loaded.document;
    let schema_version = document.schema_version;
    if !matches!(schema_version, 1..=5) {
        return Err(JourneyError::Validation(format!(
            "unsupported schema_version {schema_version}; expected 1, 2, 3, 4, or 5"
        )));
    }
    validate_id("journey", &document.journey.id)?;
    validate_nonempty("journey title", &document.journey.title)?;
    validate_nonempty("journey purpose", &document.journey.purpose)?;
    validate_nonempty("expected outcome", &document.journey.expected_outcome)?;
    if document.journey.revision == 0 {
        return Err(JourneyError::Validation(
            "journey revision must be greater than zero".to_owned(),
        ));
    }
    let total_steps = document
        .setup_steps
        .len()
        .saturating_add(document.steps.len())
        .saturating_add(document.cleanup_steps.len());
    if document.steps.is_empty() || total_steps > MAX_STEPS {
        return Err(JourneyError::Validation(format!(
            "journey must contain 1 to {MAX_STEPS} total steps"
        )));
    }
    if !document.evidence.trace {
        return Err(JourneyError::Validation(
            "trace evidence is required by schema version 1".to_owned(),
        ));
    }
    if !document.evidence.diagnostics {
        return Err(JourneyError::Validation(
            "console and page-error diagnostics are required by schema version 1".to_owned(),
        ));
    }

    if schema_version == 5 {
        if document.journey.mode != JourneyMode::Mutating {
            return Err(JourneyError::Validation(
                "journey schema version 5 requires mode = 'mutating'".to_owned(),
            ));
        }
        let fixture = document.fixture.as_ref().ok_or_else(|| {
            JourneyError::Validation(
                "journey schema version 5 requires a fixture contract".to_owned(),
            )
        })?;
        if fixture.maximum_lifetime_seconds == 0
            || fixture.maximum_lifetime_seconds > MAX_FIXTURE_LIFETIME_SECONDS
        {
            return Err(JourneyError::Validation(format!(
                "fixture maximum_lifetime_seconds must be between 1 and {MAX_FIXTURE_LIFETIME_SECONDS}"
            )));
        }
        if document.setup_steps.is_empty() || document.cleanup_steps.is_empty() {
            return Err(JourneyError::Validation(
                "mutating journeys require declared setup_steps and cleanup_steps".to_owned(),
            ));
        }
    } else {
        if document.journey.mode != JourneyMode::ReadOnly {
            return Err(JourneyError::Validation(
                "journey schema versions 1 through 4 require mode = 'read_only'".to_owned(),
            ));
        }
        if document.fixture.is_some()
            || !document.setup_steps.is_empty()
            || !document.cleanup_steps.is_empty()
        {
            return Err(JourneyError::Validation(
                "fixture phases require journey schema version 5".to_owned(),
            ));
        }
    }

    validate_authentication_fields(schema_version, document.authentication.as_ref())?;

    let origin = Origin::parse(&document.target.origin)?;
    let base = origin.base_url();
    let mut identifiers = HashSet::new();
    let mut has_checkpoint = false;
    let mut has_guide_evidence = false;
    let mut prior_checkpoints = HashSet::new();
    let setup_steps = validate_step_phase(
        schema_version,
        StepPhase::Setup,
        document.setup_steps,
        &origin,
        &base,
        &mut identifiers,
        &mut prior_checkpoints,
        &mut has_checkpoint,
        &mut has_guide_evidence,
    )?;
    let steps = validate_step_phase(
        schema_version,
        StepPhase::Journey,
        document.steps,
        &origin,
        &base,
        &mut identifiers,
        &mut prior_checkpoints,
        &mut has_checkpoint,
        &mut has_guide_evidence,
    )?;
    let cleanup_steps = validate_step_phase(
        schema_version,
        StepPhase::FixtureCleanup,
        document.cleanup_steps,
        &origin,
        &base,
        &mut identifiers,
        &mut prior_checkpoints,
        &mut has_checkpoint,
        &mut has_guide_evidence,
    )?;
    if !has_checkpoint || !has_guide_evidence {
        let message = if schema_version < 3 {
            "a journey requires at least one deterministic checkpoint and one focused capture"
        } else {
            "a journey requires at least one deterministic checkpoint and one focused evidence action"
        };
        return Err(JourneyError::Validation(message.to_owned()));
    }

    let all_steps = setup_steps
        .iter()
        .chain(steps.iter())
        .chain(cleanup_steps.iter())
        .cloned()
        .collect::<Vec<_>>();
    validate_authentication_steps(schema_version, document.authentication.as_ref(), &all_steps)?;
    if schema_version == 5 {
        let verification_step = document
            .authentication
            .as_ref()
            .and_then(|authentication| authentication.verification_step.as_deref())
            .expect("v5 authentication fields were validated");
        if !setup_steps.iter().any(|step| step.id == verification_step) {
            return Err(JourneyError::Validation(
                "mutation authentication verification_step must be declared in setup_steps"
                    .to_owned(),
            ));
        }
        if !setup_steps
            .iter()
            .any(|step| matches!(step.action, ValidatedAction::CheckAbsent { .. }))
        {
            return Err(JourneyError::Validation(
                "mutating journey setup must verify that the disposable fixture is absent"
                    .to_owned(),
            ));
        }
        if !steps.iter().any(|step| step.action.is_mutating()) {
            return Err(JourneyError::Validation(
                "mutating journey main steps require at least one mutation".to_owned(),
            ));
        }
        if !cleanup_steps
            .iter()
            .any(|step| matches!(step.action, ValidatedAction::EnsureAbsent { .. }))
        {
            return Err(JourneyError::Validation(
                "mutating journey cleanup must contain an idempotent ensure_absent action"
                    .to_owned(),
            ));
        }
    }

    Ok(ValidatedJourney {
        schema_version,
        source_path: loaded.source_path,
        source_sha256: loaded.source_sha256,
        meta: document.journey,
        origin,
        authentication: document.authentication,
        fixture: document.fixture,
        evidence: document.evidence,
        setup_steps,
        steps,
        cleanup_steps,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_step_phase(
    schema_version: u8,
    phase: StepPhase,
    source: Vec<StepSpec>,
    origin: &Origin,
    base: &Url,
    identifiers: &mut HashSet<String>,
    prior_checkpoints: &mut HashSet<String>,
    has_checkpoint: &mut bool,
    has_guide_evidence: &mut bool,
) -> Result<Vec<ValidatedStep>, JourneyError> {
    let mut steps = Vec::with_capacity(source.len());
    for step in source {
        validate_id("step", &step.id)?;
        if !identifiers.insert(step.id.clone()) {
            return Err(JourneyError::Validation(format!(
                "duplicate step id '{}' across journey phases",
                step.id
            )));
        }
        validate_nonempty("step title", &step.title)?;
        if phase != StepPhase::Journey
            && (step.guide_instruction.is_some() || !step.evidence_for.is_empty())
        {
            return Err(JourneyError::Validation(format!(
                "step '{}' may not publish guide content from setup or cleanup",
                step.id
            )));
        }
        if let Some(instruction) = step.guide_instruction.as_deref()
            && (instruction.trim().is_empty()
                || (schema_version >= 2
                    && (instruction.len() > MAX_GUIDE_INSTRUCTION_BYTES
                        || has_disallowed_control(instruction))))
        {
            return Err(JourneyError::Validation(format!(
                "step '{}' guide_instruction must contain at most {MAX_GUIDE_INSTRUCTION_BYTES} bytes and no unsupported control characters",
                step.id
            )));
        }
        if schema_version == 1 && !step.evidence_for.is_empty() {
            return Err(JourneyError::Validation(format!(
                "step '{}' evidence_for requires journey schema version 2",
                step.id
            )));
        }
        let mut evidence_targets = HashSet::new();
        for checkpoint in &step.evidence_for {
            validate_id("evidence_for checkpoint", checkpoint)?;
            if !prior_checkpoints.contains(checkpoint) || !evidence_targets.insert(checkpoint) {
                return Err(JourneyError::Validation(format!(
                    "step '{}' evidence_for entries must uniquely reference earlier checkpoints",
                    step.id
                )));
            }
        }
        let action = validate_action(
            schema_version,
            phase,
            &step.id,
            step.guide_instruction.is_some(),
            step.action,
            origin,
            base,
        )?;
        let effect = if schema_version == 5 {
            let effect = step.effect.ok_or_else(|| {
                JourneyError::Validation(format!(
                    "step '{}' requires an explicit effect classification",
                    step.id
                ))
            })?;
            if (effect == StepEffect::Mutating) != action.is_mutating() {
                return Err(JourneyError::Validation(format!(
                    "step '{}' effect does not match its action",
                    step.id
                )));
            }
            effect
        } else {
            if step.effect.is_some() || action.is_mutating() {
                return Err(JourneyError::Validation(format!(
                    "step '{}' mutation fields require journey schema version 5",
                    step.id
                )));
            }
            StepEffect::ReadOnly
        };
        if matches!(
            action,
            ValidatedAction::CheckUrl { .. }
                | ValidatedAction::CheckText { .. }
                | ValidatedAction::CheckAbsent { .. }
                | ValidatedAction::ClickButton { .. }
                | ValidatedAction::EnsureAbsent { .. }
        ) {
            *has_checkpoint = true;
            prior_checkpoints.insert(step.id.clone());
        }
        if matches!(
            action,
            ValidatedAction::Capture { .. }
                | ValidatedAction::FollowLink { .. }
                | ValidatedAction::FillText { .. }
                | ValidatedAction::ClickButton { .. }
        ) && phase == StepPhase::Journey
        {
            *has_guide_evidence = true;
        }
        if !step.evidence_for.is_empty() && !matches!(action, ValidatedAction::Capture { .. }) {
            return Err(JourneyError::Validation(format!(
                "step '{}' may declare evidence_for only on a capture action",
                step.id
            )));
        }
        steps.push(ValidatedStep {
            id: step.id,
            title: step.title,
            guide_instruction: step.guide_instruction,
            evidence_for: step.evidence_for,
            effect,
            action,
        });
    }
    Ok(steps)
}

fn validate_action(
    schema_version: u8,
    phase: StepPhase,
    step_id: &str,
    has_guide_instruction: bool,
    action: StepAction,
    origin: &Origin,
    base: &Url,
) -> Result<ValidatedAction, JourneyError> {
    let action = match action {
        StepAction::Navigate { path } => ValidatedAction::Navigate {
            url: validate_path(origin, base, &path, schema_version)?,
        },
        StepAction::CheckUrl { path } => ValidatedAction::CheckUrl {
            url: validate_path(origin, base, &path, schema_version)?,
        },
        StepAction::CheckText {
            selector,
            expected,
            comparison,
        } => {
            validate_selector(&selector)?;
            validate_expected(step_id, &expected)?;
            ValidatedAction::CheckText {
                selector,
                expected,
                comparison,
            }
        }
        StepAction::Capture { selector, alt_text } => {
            validate_selector(&selector)?;
            validate_alt_text(step_id, "capture", &alt_text)?;
            ValidatedAction::Capture { selector, alt_text }
        }
        StepAction::FollowLink {
            selector,
            expected_path,
            alt_text,
        } => {
            if schema_version < 3 {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' follow_link requires journey schema version 3"
                )));
            }
            if !has_guide_instruction {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' follow_link requires guide_instruction"
                )));
            }
            validate_selector(&selector)?;
            validate_alt_text(step_id, "follow_link", &alt_text)?;
            ValidatedAction::FollowLink {
                selector,
                expected_url: validate_path(origin, base, &expected_path, schema_version)?,
                alt_text,
            }
        }
        StepAction::FillText {
            selector,
            value,
            alt_text,
        } => {
            require_v5_main(schema_version, phase, step_id, "fill_text")?;
            validate_simple_id_selector(step_id, &selector)?;
            if value != "$fixture_token" {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' fill_text value must be the generated public $fixture_token"
                )));
            }
            if !has_guide_instruction {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' fill_text requires guide_instruction"
                )));
            }
            validate_alt_text(step_id, "fill_text", &alt_text)?;
            ValidatedAction::FillText {
                selector,
                value: MutationValue::FixtureToken,
                alt_text,
            }
        }
        StepAction::ClickButton {
            selector,
            form_selector,
            action_path,
            expected_path,
            verify_selector,
            expected_text,
            alt_text,
        } => {
            require_v5_main(schema_version, phase, step_id, "click_button")?;
            validate_simple_id_selector(step_id, &selector)?;
            validate_simple_id_selector(step_id, &form_selector)?;
            validate_selector(&verify_selector)?;
            validate_expected(step_id, &expected_text)?;
            validate_alt_text(step_id, "click_button", &alt_text)?;
            if !has_guide_instruction {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' click_button requires guide_instruction"
                )));
            }
            ValidatedAction::ClickButton {
                selector,
                form_selector,
                action_url: validate_path(origin, base, &action_path, schema_version)?,
                expected_url: validate_path(origin, base, &expected_path, schema_version)?,
                verify_selector,
                expected_text,
                alt_text,
            }
        }
        StepAction::CheckAbsent {
            selector,
            expected_text,
        } => {
            if schema_version != 5 || phase == StepPhase::FixtureCleanup {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' check_absent is available only in v5 setup or main steps"
                )));
            }
            validate_selector(&selector)?;
            validate_expected(step_id, &expected_text)?;
            ValidatedAction::CheckAbsent {
                selector,
                expected_text,
            }
        }
        StepAction::EnsureAbsent {
            status_selector,
            expected_text,
            button_selector,
            form_selector,
            action_path,
            expected_path,
            alt_text,
        } => {
            if schema_version != 5 || phase != StepPhase::FixtureCleanup {
                return Err(JourneyError::Validation(format!(
                    "step '{step_id}' ensure_absent is available only in v5 cleanup_steps"
                )));
            }
            validate_selector(&status_selector)?;
            validate_simple_id_selector(step_id, &button_selector)?;
            validate_simple_id_selector(step_id, &form_selector)?;
            validate_expected(step_id, &expected_text)?;
            validate_alt_text(step_id, "ensure_absent", &alt_text)?;
            ValidatedAction::EnsureAbsent {
                status_selector,
                expected_text,
                button_selector,
                form_selector,
                action_url: validate_path(origin, base, &action_path, schema_version)?,
                expected_url: validate_path(origin, base, &expected_path, schema_version)?,
                alt_text,
            }
        }
    };
    if phase == StepPhase::Setup && action.is_mutating() {
        return Err(JourneyError::Validation(format!(
            "step '{step_id}' setup may not mutate application state"
        )));
    }
    Ok(action)
}

fn require_v5_main(
    schema_version: u8,
    phase: StepPhase,
    step_id: &str,
    action: &str,
) -> Result<(), JourneyError> {
    if schema_version == 5 && phase == StepPhase::Journey {
        Ok(())
    } else {
        Err(JourneyError::Validation(format!(
            "step '{step_id}' {action} is available only in journey v5 main steps"
        )))
    }
}

fn validate_expected(step_id: &str, value: &str) -> Result<(), JourneyError> {
    validate_nonempty("expected text", value)?;
    if value.len() > MAX_EXPECTED_BYTES || has_disallowed_control(value) {
        return Err(JourneyError::Validation(format!(
            "step '{step_id}' expected text exceeds {MAX_EXPECTED_BYTES} bytes or contains unsupported controls"
        )));
    }
    Ok(())
}

fn validate_simple_id_selector(step_id: &str, value: &str) -> Result<(), JourneyError> {
    let identifier = value.strip_prefix('#').unwrap_or_default();
    if identifier.is_empty()
        || identifier.len() > 96
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(JourneyError::Validation(format!(
            "step '{step_id}' mutation selector must be one simple #id selector"
        )));
    }
    Ok(())
}

fn validate_authentication_fields(
    schema_version: u8,
    authentication: Option<&AuthRequirement>,
) -> Result<(), JourneyError> {
    if matches!(schema_version, 4 | 5) && authentication.is_none() {
        return Err(JourneyError::Validation(format!(
            "journey schema version {schema_version} requires an authentication contract"
        )));
    }
    let Some(authentication) = authentication else {
        return Ok(());
    };
    if matches!(schema_version, 4 | 5) {
        validate_auth_identifier("authentication provider", &authentication.provider)?;
        validate_auth_identifier("authentication role", &authentication.role)?;
    } else {
        validate_nonempty("authentication provider", &authentication.provider)?;
        validate_nonempty("authentication role", &authentication.role)?;
    }
    match (schema_version, authentication.verification_step.as_deref()) {
        (4 | 5, Some(step)) => {
            validate_auth_identifier("authentication verification_step", step)?;
            if schema_version == 5 && authentication.disposable_actor != Some(true) {
                return Err(JourneyError::Validation(
                    "journey schema version 5 requires disposable_actor = true".to_owned(),
                ));
            }
            Ok(())
        }
        (4 | 5, None) => Err(JourneyError::Validation(format!(
            "journey schema version {schema_version} requires authentication verification_step"
        ))),
        (_, Some(_)) => Err(JourneyError::Validation(
            "authentication verification_step requires journey schema version 4".to_owned(),
        )),
        (_, None) => Ok(()),
    }
}

fn validate_authentication_steps(
    schema_version: u8,
    authentication: Option<&AuthRequirement>,
    steps: &[ValidatedStep],
) -> Result<(), JourneyError> {
    if !matches!(schema_version, 4 | 5) {
        return Ok(());
    }
    let verification_step = authentication
        .and_then(|value| value.verification_step.as_deref())
        .expect("schema v4 authentication fields were validated");
    let verification_index = steps
        .iter()
        .position(|step| step.id == verification_step)
        .ok_or_else(|| {
            JourneyError::Validation(
                "authentication verification_step must reference a declared step".to_owned(),
            )
        })?;
    if !matches!(
        steps[verification_index].action,
        ValidatedAction::CheckText { .. }
    ) {
        return Err(JourneyError::Validation(
            "authentication verification_step must reference a check_text step".to_owned(),
        ));
    }
    if schema_version == 5 {
        let setup_count = steps
            .iter()
            .position(|step| step.action.is_mutating())
            .unwrap_or(steps.len());
        if verification_index >= setup_count {
            return Err(JourneyError::Validation(
                "mutation authentication verification must pass before any mutating step"
                    .to_owned(),
            ));
        }
    }
    if steps[verification_index].guide_instruction.is_some()
        || steps
            .iter()
            .take(verification_index)
            .any(|step| step.guide_instruction.is_some())
        || steps.iter().take(verification_index + 1).any(|step| {
            matches!(
                step.action,
                ValidatedAction::Capture { .. } | ValidatedAction::FollowLink { .. }
            )
        })
    {
        return Err(JourneyError::Validation(
            "authentication verification must precede guide evidence and interactive actions"
                .to_owned(),
        ));
    }
    if !steps
        .iter()
        .take(verification_index)
        .any(|step| matches!(step.action, ValidatedAction::Navigate { .. }))
        || steps.iter().take(verification_index).any(|step| {
            matches!(
                step.action,
                ValidatedAction::CheckUrl { .. } | ValidatedAction::CheckText { .. }
            )
        })
    {
        return Err(JourneyError::Validation(
            "authentication verification_step must be the first checkpoint after navigation"
                .to_owned(),
        ));
    }
    Ok(())
}

pub fn parse_authorized_origin(value: &str) -> Result<Origin, JourneyError> {
    Origin::parse(value)
}

fn validate_path(
    origin: &Origin,
    base: &Url,
    value: &str,
    schema_version: u8,
) -> Result<Url, JourneyError> {
    if !value.starts_with('/') || value.starts_with("//") || value.contains('\\') {
        return Err(JourneyError::Validation(
            "step path must begin with one slash and contain no backslashes".to_owned(),
        ));
    }
    let url = base
        .join(value)
        .map_err(|error| JourneyError::Validation(format!("invalid step path: {error}")))?;
    if !origin.contains(&url) {
        return Err(JourneyError::Validation(
            "step path resolves outside the authorized origin".to_owned(),
        ));
    }
    if schema_version >= 2 && (url.query().is_some() || url.fragment().is_some()) {
        return Err(JourneyError::Validation(format!(
            "journey v{schema_version} step paths must not contain a query or fragment"
        )));
    }
    Ok(url)
}

fn validate_id(kind: &str, value: &str) -> Result<(), JourneyError> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        });
    if valid {
        Ok(())
    } else {
        Err(JourneyError::Validation(format!(
            "{kind} id '{value}' must use 1-96 lowercase letters, digits, dots, underscores, or hyphens"
        )))
    }
}

fn validate_auth_identifier(kind: &str, value: &str) -> Result<(), JourneyError> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        });
    if valid {
        Ok(())
    } else {
        Err(JourneyError::Validation(format!(
            "{kind} must use 1-96 lowercase letters, digits, dots, underscores, or hyphens"
        )))
    }
}

fn validate_nonempty(kind: &str, value: &str) -> Result<(), JourneyError> {
    if value.trim().is_empty() {
        Err(JourneyError::Validation(format!(
            "{kind} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_selector(value: &str) -> Result<(), JourneyError> {
    if value.trim().is_empty() || value.len() > MAX_SELECTOR_BYTES || value.contains('\0') {
        return Err(JourneyError::Validation(format!(
            "selector must contain 1 to {MAX_SELECTOR_BYTES} bytes and no NUL"
        )));
    }
    Ok(())
}

fn validate_alt_text(step_id: &str, action: &str, value: &str) -> Result<(), JourneyError> {
    validate_nonempty(&format!("{action} alt_text"), value)?;
    if value.len() > MAX_ALT_TEXT_BYTES || has_disallowed_control(value) {
        return Err(JourneyError::Validation(format!(
            "step '{step_id}' {action} alt_text must contain at most {MAX_ALT_TEXT_BYTES} bytes and no control characters"
        )));
    }
    Ok(())
}

fn has_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

pub fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_document() -> LoadedJourney {
        let source = br#"
schema_version = 1

[journey]
id = "demo.read-home"
revision = 1
title = "Read home"
purpose = "Verify the home page"
expected_outcome = "The heading is visible"
mode = "read_only"

[target]
origin = "http://127.0.0.1:4173"

[evidence]
trace = true
diagnostics = true

[[steps]]
id = "open"
title = "Open home"
action = { type = "navigate", path = "/" }

[[steps]]
id = "heading"
title = "Check heading"
action = { type = "check_text", selector = "h1", expected = "Hello", comparison = "exact" }

[[steps]]
id = "capture-heading"
title = "Capture heading"
action = { type = "capture", selector = "h1", alt_text = "Highlighted heading" }
"#;
        LoadedJourney {
            source_path: PathBuf::from("journey.toml"),
            source_sha256: hex_digest(source),
            document: toml::from_slice(source).unwrap(),
        }
    }

    fn valid_follow_link_document() -> LoadedJourney {
        let mut loaded = valid_document();
        loaded.document.schema_version = 3;
        loaded.document.steps[2].title = "Follow the Continue link".to_owned();
        loaded.document.steps[2].guide_instruction =
            Some("Select the highlighted Continue link.".to_owned());
        loaded.document.steps[2].action = StepAction::FollowLink {
            selector: "#continue".to_owned(),
            expected_path: "/complete".to_owned(),
            alt_text: "Continue link highlighted in red".to_owned(),
        };
        loaded
    }

    #[test]
    fn validates_and_resolves_read_only_steps() {
        let journey = validate(valid_document()).unwrap();
        assert_eq!(journey.schema_version, 1);
        assert_eq!(journey.origin.to_string(), "http://127.0.0.1:4173");
        assert!(matches!(
            &journey.steps[0].action,
            ValidatedAction::Navigate { url } if url.as_str() == "http://127.0.0.1:4173/"
        ));
    }

    #[test]
    fn default_ports_are_canonicalized_for_exact_comparison() {
        assert_eq!(
            Origin::parse("https://example.com").unwrap(),
            Origin::parse("https://example.com:443").unwrap()
        );
        assert_ne!(
            Origin::parse("http://example.com").unwrap(),
            Origin::parse("https://example.com").unwrap()
        );
        let ipv6 = Origin::parse("http://[::1]:4173").unwrap();
        assert_eq!(ipv6.host, "::1");
        assert_eq!(ipv6.to_string(), "http://[::1]:4173");
        assert_eq!(ipv6.base_url().as_str(), "http://[::1]:4173/");
        assert!(ipv6.is_literal_loopback());
        assert!(ipv6.contains(&Url::parse("http://[::1]:4173/path").unwrap()));
        assert!(!ipv6.contains(&Url::parse("http://[::1]:4174/path").unwrap()));
    }

    #[test]
    fn rejects_unsafe_origins_and_paths() {
        for value in [
            "file:///tmp/page",
            "https://user@example.com",
            "https://example.com/path",
            "https://example.com/?query=yes",
        ] {
            assert!(Origin::parse(value).is_err(), "{value}");
        }

        let mut loaded = valid_document();
        loaded.document.steps[0].action = StepAction::Navigate {
            path: "//other.example/path".to_owned(),
        };
        assert!(validate(loaded).is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_duplicate_ids() {
        let bad = br#"
schema_version = 1
unexpected = true
[journey]
id = "demo"
revision = 1
title = "Demo"
purpose = "Demo"
expected_outcome = "Demo"
mode = "read_only"
[target]
origin = "https://example.com"
[evidence]
trace = true
diagnostics = true
[[steps]]
id = "one"
title = "One"
action = { type = "navigate", path = "/" }
"#;
        assert!(toml::from_slice::<JourneyDocument>(bad).is_err());

        let unknown_action_field = br#"
schema_version = 1
[journey]
id = "demo"
revision = 1
title = "Demo"
purpose = "Demo"
expected_outcome = "Demo"
mode = "read_only"
[target]
origin = "https://example.com"
[evidence]
trace = true
diagnostics = true
[[steps]]
id = "one"
title = "One"
action = { type = "navigate", path = "/", unexpected = true }
"#;
        assert!(toml::from_slice::<JourneyDocument>(unknown_action_field).is_err());

        let mut loaded = valid_document();
        loaded.document.steps.push(loaded.document.steps[0].clone());
        assert!(validate(loaded).is_err());
    }

    #[test]
    fn capture_evidence_links_only_to_unique_earlier_checkpoints() {
        let mut valid = valid_document();
        valid.document.schema_version = 2;
        valid.document.steps[2].evidence_for = vec!["heading".to_owned()];
        assert_eq!(
            validate(valid).unwrap().steps[2].evidence_for,
            vec!["heading"]
        );

        let mut future = valid_document();
        future.document.schema_version = 2;
        future.document.steps[0].evidence_for = vec!["heading".to_owned()];
        assert!(validate(future).is_err());

        let mut duplicate = valid_document();
        duplicate.document.schema_version = 2;
        duplicate.document.steps[2].evidence_for = vec!["heading".to_owned(), "heading".to_owned()];
        assert!(validate(duplicate).is_err());

        let mut non_capture = valid_document();
        non_capture.document.schema_version = 2;
        non_capture.document.steps[1].evidence_for = vec!["heading".to_owned()];
        assert!(validate(non_capture).is_err());
    }

    #[test]
    fn v1_url_queries_remain_compatible_but_v2_rejects_redacted_operands() {
        let mut legacy = valid_document();
        legacy.document.steps[0].action = StepAction::Navigate {
            path: "/?view=legacy".to_owned(),
        };
        assert!(validate(legacy).is_ok());

        let mut current = valid_document();
        current.document.schema_version = 2;
        current.document.steps[0].action = StepAction::Navigate {
            path: "/?view=current".to_owned(),
        };
        assert!(validate(current).is_err());
    }

    #[test]
    fn v3_resolves_follow_link_and_treats_it_as_focused_guide_evidence() {
        let journey = validate(valid_follow_link_document()).unwrap();
        assert_eq!(journey.schema_version, 3);
        assert!(matches!(
            &journey.steps[2].action,
            ValidatedAction::FollowLink {
                selector,
                expected_url,
                alt_text,
            } if selector == "#continue"
                && expected_url.as_str() == "http://127.0.0.1:4173/complete"
                && alt_text == "Continue link highlighted in red"
        ));

        let mut capture_only = valid_document();
        capture_only.document.schema_version = 3;
        assert!(validate(capture_only).is_ok());
    }

    #[test]
    fn v4_authentication_requires_the_first_visible_checkpoint_before_evidence() {
        let source = include_bytes!("../examples/authenticated-pass.toml");
        let document: JourneyDocument = toml::from_slice(source).unwrap();
        let loaded = |document| LoadedJourney {
            source_path: PathBuf::from("authenticated-pass.toml"),
            source_sha256: hex_digest(source),
            document,
        };
        let valid = validate(loaded(document.clone())).unwrap();
        assert_eq!(valid.schema_version, 4);

        let mut capture_reference = document.clone();
        capture_reference
            .authentication
            .as_mut()
            .unwrap()
            .verification_step = Some("capture-viewer-access".to_owned());
        assert!(validate(loaded(capture_reference)).is_err());

        let mut evidence_first = document.clone();
        evidence_first.steps.swap(1, 2);
        assert!(validate(loaded(evidence_first)).is_err());

        let mut checkpoint_first = document.clone();
        checkpoint_first.steps.insert(
            1,
            StepSpec {
                id: "check-auth-url".to_owned(),
                title: "Check authentication URL".to_owned(),
                guide_instruction: None,
                evidence_for: Vec::new(),
                effect: None,
                action: StepAction::CheckUrl {
                    path: "/authenticated".to_owned(),
                },
            },
        );
        assert!(validate(loaded(checkpoint_first)).is_err());

        let mut legacy = document;
        legacy.schema_version = 3;
        assert!(validate(loaded(legacy)).is_err());

        let mut compatible_legacy = valid_document();
        compatible_legacy.document.schema_version = 3;
        compatible_legacy.document.authentication = Some(AuthRequirement {
            provider: "Legacy Provider".to_owned(),
            role: "Read Only Viewer".to_owned(),
            verification_step: None,
            disposable_actor: None,
        });
        assert!(validate(compatible_legacy).is_ok());
    }

    #[test]
    fn v5_authentication_verification_must_execute_during_setup() {
        let source = include_bytes!("../examples/mutating-pass.toml");
        let mut document: JourneyDocument = toml::from_slice(source).unwrap();
        let verification = document.setup_steps.remove(1);
        document.steps.insert(0, verification);
        let error = validate(LoadedJourney {
            source_path: PathBuf::from("mutating-pass.toml"),
            source_sha256: hex_digest(source),
            document,
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("verification_step must be declared in setup_steps")
        );
    }

    #[test]
    fn follow_link_requires_v3_instruction_and_a_separate_checkpoint() {
        for schema_version in [1, 2] {
            let mut legacy = valid_follow_link_document();
            legacy.document.schema_version = schema_version;
            assert!(validate(legacy).is_err(), "schema v{schema_version}");
        }

        let mut missing_instruction = valid_follow_link_document();
        missing_instruction.document.steps[2].guide_instruction = None;
        assert!(validate(missing_instruction).is_err());

        let mut empty_instruction = valid_follow_link_document();
        empty_instruction.document.steps[2].guide_instruction = Some(" \t".to_owned());
        assert!(validate(empty_instruction).is_err());

        for guide_instruction in [
            "x".repeat(MAX_GUIDE_INSTRUCTION_BYTES + 1),
            "Select\0 Continue".to_owned(),
        ] {
            let mut invalid_instruction = valid_follow_link_document();
            invalid_instruction.document.steps[2].guide_instruction = Some(guide_instruction);
            assert!(validate(invalid_instruction).is_err());
        }

        let mut no_checkpoint = valid_follow_link_document();
        no_checkpoint.document.steps.remove(1);
        assert!(validate(no_checkpoint).is_err());
    }

    #[test]
    fn follow_link_rejects_unsafe_paths_selectors_alt_text_and_evidence_links() {
        for expected_path in [
            "https://example.com/complete",
            "//example.com/complete",
            "/complete?token=secret",
            "/complete#fragment",
            r"/complete\backslash",
        ] {
            let mut loaded = valid_follow_link_document();
            loaded.document.steps[2].action = StepAction::FollowLink {
                selector: "#continue".to_owned(),
                expected_path: expected_path.to_owned(),
                alt_text: "Continue link".to_owned(),
            };
            assert!(validate(loaded).is_err(), "{expected_path}");
        }

        for selector in [
            String::new(),
            "x".repeat(MAX_SELECTOR_BYTES + 1),
            "#bad\0".to_owned(),
        ] {
            let mut loaded = valid_follow_link_document();
            loaded.document.steps[2].action = StepAction::FollowLink {
                selector,
                expected_path: "/complete".to_owned(),
                alt_text: "Continue link".to_owned(),
            };
            assert!(validate(loaded).is_err());
        }

        for alt_text in [
            String::new(),
            "x".repeat(MAX_ALT_TEXT_BYTES + 1),
            "bad\u{0}alt".to_owned(),
        ] {
            let mut loaded = valid_follow_link_document();
            loaded.document.steps[2].action = StepAction::FollowLink {
                selector: "#continue".to_owned(),
                expected_path: "/complete".to_owned(),
                alt_text,
            };
            assert!(validate(loaded).is_err());
        }

        let mut associated = valid_follow_link_document();
        associated.document.steps[2].evidence_for = vec!["heading".to_owned()];
        assert!(validate(associated).is_err());
    }

    #[test]
    fn published_v3_schema_accepts_follow_link_and_rejects_missing_instruction() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/journey-v3.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let mut document = serde_json::json!({
            "schema_version": 3,
            "journey": {
                "id": "demo.follow-link",
                "revision": 1,
                "title": "Follow a link",
                "purpose": "Exercise one visible link",
                "expected_outcome": "The destination is visible",
                "mode": "read_only"
            },
            "target": { "origin": "http://127.0.0.1:4173" },
            "evidence": { "trace": true, "diagnostics": true },
            "steps": [
                {
                    "id": "open",
                    "title": "Open home",
                    "action": { "type": "navigate", "path": "/" }
                },
                {
                    "id": "heading",
                    "title": "Check heading",
                    "action": {
                        "type": "check_text",
                        "selector": "h1",
                        "expected": "Hello",
                        "comparison": "exact"
                    }
                },
                {
                    "id": "continue",
                    "title": "Follow Continue",
                    "guide_instruction": "Select Continue.",
                    "action": {
                        "type": "follow_link",
                        "selector": "#continue",
                        "expected_path": "/complete",
                        "alt_text": "Continue highlighted in red"
                    }
                }
            ]
        });
        assert!(validator.is_valid(&document));

        document["steps"][2]
            .as_object_mut()
            .unwrap()
            .remove("guide_instruction");
        assert!(!validator.is_valid(&document));
    }
}
