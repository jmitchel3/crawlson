use std::collections::HashSet;
use std::fmt;
use std::fs;
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JourneyDocument {
    pub schema_version: u8,
    pub journey: JourneyMeta,
    pub target: TargetSpec,
    #[serde(default)]
    pub authentication: Option<AuthRequirement>,
    pub evidence: EvidencePolicy,
    pub steps: Vec<StepSpec>,
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
    pub action: StepAction,
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
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub meta: JourneyMeta,
    pub origin: Origin,
    pub authentication: Option<AuthRequirement>,
    pub evidence: EvidencePolicy,
    pub steps: Vec<ValidatedStep>,
}

#[derive(Debug, Clone)]
pub struct ValidatedStep {
    pub id: String,
    pub title: String,
    pub guide_instruction: Option<String>,
    pub action: ValidatedAction,
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
    let metadata = fs::metadata(path).map_err(|error| JourneyError::Read(error.to_string()))?;
    if !metadata.is_file() {
        return Err(JourneyError::Read("path is not a regular file".to_owned()));
    }
    if metadata.len() > MAX_JOURNEY_BYTES {
        return Err(JourneyError::Read(format!(
            "file exceeds {MAX_JOURNEY_BYTES} bytes"
        )));
    }
    let bytes = fs::read(path).map_err(|error| JourneyError::Read(error.to_string()))?;
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
    if document.schema_version != 1 {
        return Err(JourneyError::Validation(format!(
            "unsupported schema_version {}; expected 1",
            document.schema_version
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
    if document.steps.is_empty() || document.steps.len() > MAX_STEPS {
        return Err(JourneyError::Validation(format!(
            "journey must contain 1 to {MAX_STEPS} steps"
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

    if let Some(authentication) = &document.authentication {
        validate_nonempty("authentication provider", &authentication.provider)?;
        validate_nonempty("authentication role", &authentication.role)?;
    }

    let origin = Origin::parse(&document.target.origin)?;
    let base = origin.base_url();
    let mut identifiers = HashSet::new();
    let mut steps = Vec::with_capacity(document.steps.len());
    let mut has_checkpoint = false;
    let mut has_capture = false;
    for step in document.steps {
        validate_id("step", &step.id)?;
        if !identifiers.insert(step.id.clone()) {
            return Err(JourneyError::Validation(format!(
                "duplicate step id '{}'",
                step.id
            )));
        }
        validate_nonempty("step title", &step.title)?;
        if step
            .guide_instruction
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(JourneyError::Validation(format!(
                "step '{}' has an empty guide_instruction",
                step.id
            )));
        }
        let action = match step.action {
            StepAction::Navigate { path } => ValidatedAction::Navigate {
                url: validate_path(&origin, &base, &path)?,
            },
            StepAction::CheckUrl { path } => ValidatedAction::CheckUrl {
                url: {
                    has_checkpoint = true;
                    validate_path(&origin, &base, &path)?
                },
            },
            StepAction::CheckText {
                selector,
                expected,
                comparison,
            } => {
                validate_selector(&selector)?;
                validate_nonempty("expected text", &expected)?;
                if expected.len() > MAX_EXPECTED_BYTES {
                    return Err(JourneyError::Validation(format!(
                        "step '{}' expected text exceeds {MAX_EXPECTED_BYTES} bytes",
                        step.id
                    )));
                }
                has_checkpoint = true;
                ValidatedAction::CheckText {
                    selector,
                    expected,
                    comparison,
                }
            }
            StepAction::Capture { selector, alt_text } => {
                validate_selector(&selector)?;
                validate_nonempty("capture alt_text", &alt_text)?;
                if alt_text.len() > MAX_ALT_TEXT_BYTES
                    || alt_text.chars().any(|character| {
                        character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                    })
                {
                    return Err(JourneyError::Validation(format!(
                        "step '{}' capture alt_text must contain at most {MAX_ALT_TEXT_BYTES} bytes and no control characters",
                        step.id
                    )));
                }
                has_capture = true;
                ValidatedAction::Capture { selector, alt_text }
            }
        };
        steps.push(ValidatedStep {
            id: step.id,
            title: step.title,
            guide_instruction: step.guide_instruction,
            action,
        });
    }
    if !has_checkpoint || !has_capture {
        return Err(JourneyError::Validation(
            "a journey requires at least one deterministic checkpoint and one focused capture"
                .to_owned(),
        ));
    }

    Ok(ValidatedJourney {
        source_path: loaded.source_path,
        source_sha256: loaded.source_sha256,
        meta: document.journey,
        origin,
        authentication: document.authentication,
        evidence: document.evidence,
        steps,
    })
}

pub fn parse_authorized_origin(value: &str) -> Result<Origin, JourneyError> {
    Origin::parse(value)
}

fn validate_path(origin: &Origin, base: &Url, value: &str) -> Result<Url, JourneyError> {
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

    #[test]
    fn validates_and_resolves_read_only_steps() {
        let journey = validate(valid_document()).unwrap();
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
}
