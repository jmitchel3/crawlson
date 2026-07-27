use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;
use wait_timeout::ChildExt;

use crate::focus::{CssBox, Viewport};
use crate::journey::{Origin, hex_digest};
use crate::net_guard::ExactOriginGuard;

const MAX_STDOUT_BYTES: usize = 1_048_576;
const MAX_STDERR_BYTES: usize = 65_536;
const MAX_SCREENSHOT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TRACE_BYTES: u64 = 256 * 1024 * 1024;
const OWNED_CONFIG: &[u8] =
    b"{\"headed\":false,\"noAutoDialog\":true,\"screenshotFormat\":\"png\"}\n";
const READ_ONLY_POLICY: &[u8] = b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\"]}\n";
const FOLLOW_LINK_POLICY: &[u8] = b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\",\"getattribute\",\"isenabled\",\"click\"]}\n";
const AUTHENTICATED_READ_ONLY_POLICY: &[u8] = b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\",\"state_load\"]}\n";
const AUTHENTICATED_FOLLOW_LINK_POLICY: &[u8] = b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\",\"getattribute\",\"isenabled\",\"click\",\"state_load\"]}\n";
const MUTATION_POLICY: &[u8] = b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\",\"getattribute\",\"isenabled\",\"count\",\"inputvalue\",\"fill\",\"click\"]}\n";
const AUTHENTICATED_MUTATION_POLICY: &[u8] = b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\",\"getattribute\",\"isenabled\",\"count\",\"inputvalue\",\"fill\",\"click\",\"state_load\"]}\n";
pub const VIEWPORT_WIDTH_CSS: f64 = 1280.0;
pub const VIEWPORT_HEIGHT_CSS: f64 = 720.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverPolicyMode {
    ReadOnly,
    FollowLink,
    AuthenticatedReadOnly,
    AuthenticatedFollowLink,
    Mutation,
    AuthenticatedMutation,
}

impl DriverPolicyMode {
    fn policy(self) -> &'static [u8] {
        match self {
            Self::ReadOnly => READ_ONLY_POLICY,
            Self::FollowLink => FOLLOW_LINK_POLICY,
            Self::AuthenticatedReadOnly => AUTHENTICATED_READ_ONLY_POLICY,
            Self::AuthenticatedFollowLink => AUTHENTICATED_FOLLOW_LINK_POLICY,
            Self::Mutation => MUTATION_POLICY,
            Self::AuthenticatedMutation => AUTHENTICATED_MUTATION_POLICY,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverCommandRecord {
    pub sequence: u32,
    pub capability: String,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
    pub upstream_success: bool,
    pub stdout_bytes: usize,
    pub stdout_captured_bytes: usize,
    pub stdout_captured_sha256: String,
    pub stderr_bytes: usize,
    pub stderr_captured_bytes: usize,
    pub stderr_captured_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsSummary {
    pub console_messages: usize,
    pub console_sha256: String,
    pub page_errors: usize,
    pub page_errors_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureBundle {
    pub raw_path: PathBuf,
    pub target: CssBox,
    pub viewport: Viewport,
    pub capture_token: String,
    pub box_command_sequence: u32,
    pub screenshot_command_sequence: u32,
}

#[derive(Debug, Error, Clone)]
pub enum DriverError {
    #[error("agent-browser is unavailable: {0}")]
    Unavailable(String),
    #[error("agent-browser command '{capability}' timed out after {seconds} seconds")]
    Timeout { capability: String, seconds: u64 },
    #[error("agent-browser command '{0}' exceeded its output limit")]
    OutputLimit(String),
    #[error("agent-browser command '{capability}' failed: {message}")]
    CommandFailed { capability: String, message: String },
    #[error("agent-browser blocked navigation: {0}")]
    NavigationBlocked(String),
    #[error("agent-browser command '{capability}' required confirmation and was not executed")]
    ConfirmationRequired { capability: String },
    #[error("browser action effect is unknown after dispatch")]
    ActionEffectUnknown(String),
    #[error("agent-browser protocol error for '{capability}': {message}")]
    Protocol { capability: String, message: String },
    #[error("agent-browser artifact error: {0}")]
    Artifact(String),
    #[error("agent-browser I/O error: {0}")]
    Io(String),
}

pub trait BrowserDriver {
    fn prepare(&mut self) -> Result<(), DriverError>;
    fn load_authentication(&mut self, _path: &Path) -> Result<(), DriverError> {
        Err(DriverError::Protocol {
            capability: "authentication_load".to_owned(),
            message: "driver does not implement authentication state loading".to_owned(),
        })
    }
    fn start_trace(&mut self) -> Result<(), DriverError>;
    fn verify_exact_origin_guard(&mut self) -> Result<u32, DriverError> {
        Err(DriverError::Protocol {
            capability: "exact_origin_guard".to_owned(),
            message: "driver does not implement exact-origin guard attestation".to_owned(),
        })
    }
    fn begin_fixture_cleanup(&mut self) {}
    fn navigate(&mut self, url: &Url) -> Result<(), DriverError>;
    fn current_url(&mut self) -> Result<Url, DriverError>;
    fn text(&mut self, selector: &str) -> Result<String, DriverError>;
    fn visible(&mut self, selector: &str) -> Result<bool, DriverError>;
    fn enabled(&mut self, _selector: &str) -> Result<bool, DriverError> {
        Err(DriverError::Protocol {
            capability: "is_enabled".to_owned(),
            message: "driver does not implement enabled-state inspection".to_owned(),
        })
    }
    fn attribute(&mut self, _selector: &str, _name: &str) -> Result<Option<String>, DriverError> {
        Err(DriverError::Protocol {
            capability: "get_attribute".to_owned(),
            message: "driver does not implement attribute inspection".to_owned(),
        })
    }
    fn count(&mut self, _selector: &str) -> Result<u64, DriverError> {
        Err(DriverError::Protocol {
            capability: "get_count".to_owned(),
            message: "driver does not implement element counting".to_owned(),
        })
    }
    fn value(&mut self, _selector: &str) -> Result<String, DriverError> {
        Err(DriverError::Protocol {
            capability: "get_value".to_owned(),
            message: "driver does not implement value inspection".to_owned(),
        })
    }
    fn fill(&mut self, _selector: &str, _value: &str) -> Result<(), DriverError> {
        Err(DriverError::Protocol {
            capability: "fill".to_owned(),
            message: "driver does not implement fill".to_owned(),
        })
    }
    fn click(&mut self, _selector: &str) -> Result<(), DriverError> {
        Err(DriverError::Protocol {
            capability: "click".to_owned(),
            message: "driver does not implement click".to_owned(),
        })
    }
    fn capture(&mut self, selector: &str, path: &Path) -> Result<CaptureBundle, DriverError>;
    fn diagnostics(&mut self) -> Result<DiagnosticsSummary, DriverError>;
    fn stop_trace(&mut self, path: &Path) -> Result<PathBuf, DriverError>;
    fn close(&mut self) -> Result<(), DriverError>;
    fn records(&self) -> Vec<DriverCommandRecord>;
}

pub struct AgentBrowserDriver {
    executable: PathBuf,
    run_root: PathBuf,
    origin: Origin,
    session: String,
    config_path: PathBuf,
    policy_path: PathBuf,
    policy: &'static [u8],
    exact_origin_guard: Option<ExactOriginGuard>,
    browser_executable: Option<PathBuf>,
    timeout: Duration,
    run_deadline: Instant,
    trace_started: bool,
    viewport: Option<Viewport>,
    records: Vec<DriverCommandRecord>,
}

impl AgentBrowserDriver {
    pub fn new(
        executable: &Path,
        run_root: &Path,
        origin: Origin,
        session: String,
        timeout: Duration,
        run_timeout: Duration,
    ) -> Result<Self, DriverError> {
        Self::new_with_policy_mode(
            executable,
            run_root,
            origin,
            session,
            timeout,
            run_timeout,
            DriverPolicyMode::ReadOnly,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_policy_mode(
        executable: &Path,
        run_root: &Path,
        origin: Origin,
        session: String,
        timeout: Duration,
        run_timeout: Duration,
        policy_mode: DriverPolicyMode,
    ) -> Result<Self, DriverError> {
        Self::new_with_policy_mode_and_browser(
            executable,
            run_root,
            origin,
            session,
            timeout,
            run_timeout,
            policy_mode,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_policy_mode_and_browser(
        executable: &Path,
        run_root: &Path,
        origin: Origin,
        session: String,
        timeout: Duration,
        run_timeout: Duration,
        policy_mode: DriverPolicyMode,
        browser_executable: Option<&Path>,
    ) -> Result<Self, DriverError> {
        if timeout.is_zero() || timeout >= Duration::from_secs(30) {
            return Err(DriverError::Io(
                "action timeout must be greater than zero and below 30 seconds".to_owned(),
            ));
        }
        if run_timeout < Duration::from_secs(30) || run_timeout > Duration::from_secs(3_600) {
            return Err(DriverError::Io(
                "run timeout must be between 30 and 3600 seconds".to_owned(),
            ));
        }
        let executable = executable
            .canonicalize()
            .map_err(|error| DriverError::Unavailable(error.to_string()))?;
        let run_root = run_root
            .canonicalize()
            .map_err(|error| DriverError::Io(error.to_string()))?;
        if !valid_session(&session) {
            return Err(DriverError::Io(
                "generated session name is invalid".to_owned(),
            ));
        }
        let control = run_root.join("control");
        fs::create_dir_all(&control).map_err(|error| DriverError::Io(error.to_string()))?;
        let config_path = control.join("agent-browser.json");
        let policy_path = control.join(match policy_mode {
            DriverPolicyMode::ReadOnly => "read-only-policy.json",
            DriverPolicyMode::FollowLink => "follow-link-policy.json",
            DriverPolicyMode::AuthenticatedReadOnly => "authenticated-read-only-policy.json",
            DriverPolicyMode::AuthenticatedFollowLink => "authenticated-follow-link-policy.json",
            DriverPolicyMode::Mutation => "mutation-policy.json",
            DriverPolicyMode::AuthenticatedMutation => "authenticated-mutation-policy.json",
        });
        let policy = policy_mode.policy();
        fs::write(&config_path, OWNED_CONFIG)
            .map_err(|error| DriverError::Io(error.to_string()))?;
        fs::write(&policy_path, policy).map_err(|error| DriverError::Io(error.to_string()))?;
        let exact_origin_guard = matches!(
            policy_mode,
            DriverPolicyMode::Mutation | DriverPolicyMode::AuthenticatedMutation
        )
        .then(|| ExactOriginGuard::materialize(&run_root, &origin))
        .transpose()
        .map_err(|error| DriverError::Io(error.to_string()))?;
        let browser_executable = browser_executable
            .map(validate_extension_browser)
            .transpose()?;
        if exact_origin_guard.is_some() && browser_executable.is_none() {
            return Err(DriverError::Unavailable(
                "mutating journeys require an explicit extension-capable Chromium or Chrome for Testing executable"
                    .to_owned(),
            ));
        }

        Ok(Self {
            executable,
            run_root,
            origin,
            session,
            config_path,
            policy_path,
            policy,
            exact_origin_guard,
            browser_executable,
            timeout,
            run_deadline: Instant::now() + run_timeout,
            trace_started: false,
            viewport: None,
            records: Vec::new(),
        })
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    fn execute(&mut self, capability: &str, arguments: &[&str]) -> Result<Value, DriverError> {
        let timeout = command_timeout(capability, self.timeout, self.run_deadline, Instant::now())
            .ok_or_else(|| DriverError::Timeout {
                capability: "run_deadline".to_owned(),
                seconds: 0,
            })?;
        let mut command = Command::new(&self.executable);
        command
            .env_clear()
            .args(["--session", &self.session])
            .arg("--json")
            .arg("--config")
            .arg(&self.config_path)
            .args(["--allowed-domains", &self.origin.host])
            .arg("--action-policy")
            .arg(&self.policy_path)
            .arg("--content-boundaries")
            .args(["--max-output", "65536"])
            .args(["--headed", "false"])
            .arg("--no-auto-dialog")
            .args(["--screenshot-format", "png"]);
        if let Some(guard) = &self.exact_origin_guard {
            command.arg("--extension").arg(guard.extension_path());
        }
        if let Some(executable) = &self.browser_executable {
            command.arg("--executable-path").arg(executable);
        }
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        add_safe_environment(&mut command, timeout);

        let start = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|error| DriverError::Unavailable(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| DriverError::Io("stdout pipe was unavailable".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| DriverError::Io("stderr pipe was unavailable".to_owned()))?;
        let stdout_reader = thread::spawn(move || read_limited(stdout, MAX_STDOUT_BYTES));
        let stderr_reader = thread::spawn(move || read_limited(stderr, MAX_STDERR_BYTES));

        let status = match child.wait_timeout(timeout) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_reader(stdout_reader)?;
                let stderr = join_reader(stderr_reader)?;
                self.record(capability, start, None, false, &stdout, &stderr);
                return Err(DriverError::Timeout {
                    capability: capability.to_owned(),
                    seconds: timeout.as_secs().max(1),
                });
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_reader(stdout_reader)?;
                let stderr = join_reader(stderr_reader)?;
                self.record(capability, start, None, false, &stdout, &stderr);
                return Err(DriverError::Io(error.to_string()));
            }
        };
        let stdout = join_reader(stdout_reader)?;
        let stderr = join_reader(stderr_reader)?;
        if stdout.oversized || stderr.oversized {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            return Err(DriverError::OutputLimit(capability.to_owned()));
        }
        let envelope: Envelope = serde_json::from_slice(&stdout.bytes).map_err(|error| {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            DriverError::Protocol {
                capability: capability.to_owned(),
                message: format!("invalid JSON response: {error}"),
            }
        })?;
        let confirmation_required = envelope.data.as_ref().is_some_and(|data| {
            data.get("confirmation_required")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
        if confirmation_required {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            return Err(DriverError::ConfirmationRequired {
                capability: capability.to_owned(),
            });
        }
        if status.success() != envelope.success {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            return Err(DriverError::Protocol {
                capability: capability.to_owned(),
                message: "process exit status contradicted the JSON envelope".to_owned(),
            });
        }
        if envelope.success && envelope.error.is_some() {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            return Err(DriverError::Protocol {
                capability: capability.to_owned(),
                message: "successful response included an error".to_owned(),
            });
        }
        if !envelope.success
            && (envelope.data.is_some()
                || envelope
                    .error
                    .as_deref()
                    .is_none_or(|message| message.trim().is_empty()))
        {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            return Err(DriverError::Protocol {
                capability: capability.to_owned(),
                message: "failed response had a contradictory envelope shape".to_owned(),
            });
        }
        if !envelope.success {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            let message = envelope
                .error
                .unwrap_or_else(|| "unknown failure".to_owned());
            if capability == "navigate" && navigation_was_blocked(&message) {
                return Err(DriverError::NavigationBlocked(message));
            }
            return Err(DriverError::CommandFailed {
                capability: capability.to_owned(),
                message,
            });
        }
        let data = envelope.data.ok_or_else(|| {
            self.record(capability, start, Some(status), false, &stdout, &stderr);
            DriverError::Protocol {
                capability: capability.to_owned(),
                message: "successful response omitted data".to_owned(),
            }
        })?;
        self.record(capability, start, Some(status), true, &stdout, &stderr);
        Ok(data)
    }

    fn record(
        &mut self,
        capability: &str,
        start: Instant,
        status: Option<ExitStatus>,
        upstream_success: bool,
        stdout: &LimitedRead,
        stderr: &LimitedRead,
    ) {
        let sensitive = capability == "authentication_load";
        self.records.push(DriverCommandRecord {
            sequence: self.records.len() as u32 + 1,
            capability: capability.to_owned(),
            duration_ms: duration_ms(start.elapsed()),
            exit_code: status.and_then(|value| value.code()),
            upstream_success,
            stdout_bytes: if sensitive { 0 } else { stdout.total },
            stdout_captured_bytes: if sensitive { 0 } else { stdout.bytes.len() },
            stdout_captured_sha256: hex_digest(if sensitive { &[] } else { &stdout.bytes }),
            stderr_bytes: if sensitive { 0 } else { stderr.total },
            stderr_captured_bytes: if sensitive { 0 } else { stderr.bytes.len() },
            stderr_captured_sha256: hex_digest(if sensitive { &[] } else { &stderr.bytes }),
        });
    }

    fn artifact(&self, returned: &str, expected: &Path, max: u64) -> Result<PathBuf, DriverError> {
        let returned = PathBuf::from(returned);
        let canonical = returned
            .canonicalize()
            .map_err(|error| DriverError::Artifact(error.to_string()))?;
        let expected = expected
            .canonicalize()
            .map_err(|error| DriverError::Artifact(error.to_string()))?;
        if canonical != expected || !canonical.starts_with(&self.run_root) {
            return Err(DriverError::Artifact(
                "driver returned an unexpected or out-of-run path".to_owned(),
            ));
        }
        let metadata =
            fs::metadata(&canonical).map_err(|error| DriverError::Artifact(error.to_string()))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max {
            return Err(DriverError::Artifact(format!(
                "artifact must be a non-empty regular file no larger than {max} bytes"
            )));
        }
        Ok(canonical)
    }

    fn validate_response_origin(&self, capability: &str, data: &Value) -> Result<(), DriverError> {
        let response_url = data
            .get("origin")
            .and_then(Value::as_str)
            .ok_or_else(|| DriverError::Protocol {
                capability: capability.to_owned(),
                message: "response omitted origin URL".to_owned(),
            })
            .and_then(|value| {
                Url::parse(value).map_err(|_| DriverError::Protocol {
                    capability: capability.to_owned(),
                    message: "response origin URL was invalid".to_owned(),
                })
            })?;
        if !self.origin.contains(&response_url) {
            return Err(DriverError::NavigationBlocked(format!(
                "{capability} response was outside the authorized origin"
            )));
        }
        Ok(())
    }
}

impl BrowserDriver for AgentBrowserDriver {
    fn prepare(&mut self) -> Result<(), DriverError> {
        if fs::read(&self.config_path).ok().as_deref() != Some(OWNED_CONFIG)
            || fs::read(&self.policy_path).ok().as_deref() != Some(self.policy)
        {
            return Err(DriverError::Protocol {
                capability: "launch_policy".to_owned(),
                message: "owned configuration or action policy changed before launch".to_owned(),
            });
        }
        if let Some(guard) = &self.exact_origin_guard {
            guard.verify().map_err(|error| DriverError::Protocol {
                capability: "exact_origin_guard".to_owned(),
                message: error.to_string(),
            })?;
        }
        let data = self.execute("set_viewport", &["set", "viewport", "1280", "720"])?;
        if data.get("width").and_then(Value::as_u64) != Some(1280)
            || data.get("height").and_then(Value::as_u64) != Some(720)
            || data.get("deviceScaleFactor").and_then(Value::as_f64) != Some(1.0)
            || data.get("mobile").and_then(Value::as_bool) != Some(false)
        {
            return Err(DriverError::Protocol {
                capability: "set_viewport".to_owned(),
                message: "response did not confirm the required 1280x720 CSS viewport at scale 1"
                    .to_owned(),
            });
        }
        self.viewport = Some(Viewport {
            width_css: VIEWPORT_WIDTH_CSS,
            height_css: VIEWPORT_HEIGHT_CSS,
            device_scale_factor: 1.0,
            scroll_x_css: None,
            scroll_y_css: None,
        });
        Ok(())
    }

    fn load_authentication(&mut self, path: &Path) -> Result<(), DriverError> {
        let value = path.to_str().ok_or_else(|| {
            DriverError::Io("temporary authentication state path is not valid UTF-8".to_owned())
        })?;
        let data = self.execute("authentication_load", &["state", "load", value])?;
        if data.get("loaded").and_then(Value::as_bool) != Some(true)
            || data.get("path").and_then(Value::as_str) != Some(value)
        {
            return Err(DriverError::Protocol {
                capability: "authentication_load".to_owned(),
                message: "response did not confirm the exact temporary state path".to_owned(),
            });
        }
        Ok(())
    }

    fn start_trace(&mut self) -> Result<(), DriverError> {
        let data = self.execute("trace_start", &["trace", "start"])?;
        if data.get("started").and_then(Value::as_bool) != Some(true) {
            return Err(DriverError::Protocol {
                capability: "trace_start".to_owned(),
                message: "response did not confirm tracing started".to_owned(),
            });
        }
        self.trace_started = true;
        Ok(())
    }

    fn verify_exact_origin_guard(&mut self) -> Result<u32, DriverError> {
        let selector = self
            .exact_origin_guard
            .as_ref()
            .ok_or_else(|| DriverError::Protocol {
                capability: "exact_origin_guard".to_owned(),
                message: "mutating driver omitted its exact-origin guard".to_owned(),
            })?
            .marker_selector()
            .to_owned();
        let data = self.execute("exact_origin_guard", &["get", "count", &selector])?;
        if data.get("count").and_then(Value::as_u64) != Some(1) {
            return Err(DriverError::Protocol {
                capability: "exact_origin_guard".to_owned(),
                message: "browser did not attest the owned exact-origin network guard".to_owned(),
            });
        }
        self.records
            .last()
            .filter(|record| record.capability == "exact_origin_guard")
            .map(|record| record.sequence)
            .ok_or_else(|| DriverError::Protocol {
                capability: "exact_origin_guard".to_owned(),
                message: "guard attestation omitted command provenance".to_owned(),
            })
    }

    fn begin_fixture_cleanup(&mut self) {
        self.run_deadline = Instant::now() + Duration::from_secs(60);
    }

    fn navigate(&mut self, url: &Url) -> Result<(), DriverError> {
        let data = self.execute("navigate", &["open", url.as_str()])?;
        let returned =
            data.get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| DriverError::Protocol {
                    capability: "navigate".to_owned(),
                    message: "response omitted URL".to_owned(),
                })?;
        let returned = Url::parse(returned).map_err(|_| DriverError::Protocol {
            capability: "navigate".to_owned(),
            message: "response URL was invalid".to_owned(),
        })?;
        if !self.origin.contains(&returned) {
            return Err(DriverError::NavigationBlocked(
                "navigation response was outside the authorized origin".to_owned(),
            ));
        }
        Ok(())
    }

    fn current_url(&mut self) -> Result<Url, DriverError> {
        let data = self.execute("current_url", &["get", "url"])?;
        let value =
            data.get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| DriverError::Protocol {
                    capability: "current_url".to_owned(),
                    message: "response omitted URL".to_owned(),
                })?;
        Url::parse(value).map_err(|error| DriverError::Protocol {
            capability: "current_url".to_owned(),
            message: format!("response URL was invalid: {error}"),
        })
    }

    fn text(&mut self, selector: &str) -> Result<String, DriverError> {
        let data = self.execute("text", &["get", "text", selector])?;
        self.validate_response_origin("text", &data)?;
        data.get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| DriverError::Protocol {
                capability: "text".to_owned(),
                message: "response omitted text".to_owned(),
            })
    }

    fn visible(&mut self, selector: &str) -> Result<bool, DriverError> {
        let data = self.execute("is_visible", &["is", "visible", selector])?;
        self.validate_response_origin("is_visible", &data)?;
        data.get("visible")
            .and_then(Value::as_bool)
            .ok_or_else(|| DriverError::Protocol {
                capability: "is_visible".to_owned(),
                message: "response omitted visible boolean".to_owned(),
            })
    }

    fn enabled(&mut self, selector: &str) -> Result<bool, DriverError> {
        let data = self.execute("is_enabled", &["is", "enabled", selector])?;
        self.validate_response_origin("is_enabled", &data)?;
        data.get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| DriverError::Protocol {
                capability: "is_enabled".to_owned(),
                message: "response omitted enabled boolean".to_owned(),
            })
    }

    fn attribute(&mut self, selector: &str, name: &str) -> Result<Option<String>, DriverError> {
        let data = self.execute("get_attribute", &["get", "attr", selector, name])?;
        self.validate_response_origin("get_attribute", &data)?;
        match data.get("value") {
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(Value::Null) => Ok(None),
            _ => Err(DriverError::Protocol {
                capability: "get_attribute".to_owned(),
                message: "response omitted string-or-null attribute value".to_owned(),
            }),
        }
    }

    fn count(&mut self, selector: &str) -> Result<u64, DriverError> {
        let data = self.execute("get_count", &["get", "count", selector])?;
        data.get("count")
            .and_then(Value::as_u64)
            .ok_or_else(|| DriverError::Protocol {
                capability: "get_count".to_owned(),
                message: "response omitted element count".to_owned(),
            })
    }

    fn value(&mut self, selector: &str) -> Result<String, DriverError> {
        let data = self.execute("get_value", &["get", "value", selector])?;
        data.get("value")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| DriverError::Protocol {
                capability: "get_value".to_owned(),
                message: "response omitted string value".to_owned(),
            })
    }

    fn fill(&mut self, selector: &str, value: &str) -> Result<(), DriverError> {
        let data = self.execute("fill", &["fill", selector, value])?;
        if data.get("filled").and_then(Value::as_str) != Some(selector) {
            return Err(DriverError::Protocol {
                capability: "fill".to_owned(),
                message: "response did not acknowledge the exact selector".to_owned(),
            });
        }
        Ok(())
    }

    fn click(&mut self, selector: &str) -> Result<(), DriverError> {
        let data = self.execute("click", &["click", selector])?;
        validate_click_response(selector, &data)
    }

    fn capture(&mut self, selector: &str, path: &Path) -> Result<CaptureBundle, DriverError> {
        ensure_output_parent(&self.run_root, path)?;
        let viewport = self.viewport.ok_or_else(|| DriverError::Protocol {
            capability: "capture".to_owned(),
            message: "capture requested before viewport confirmation".to_owned(),
        })?;
        let data = self.execute("bounding_box", &["get", "box", selector])?;
        let number = |name: &str| {
            data.get(name)
                .and_then(Value::as_f64)
                .ok_or_else(|| DriverError::Protocol {
                    capability: "bounding_box".to_owned(),
                    message: format!("response omitted numeric {name}"),
                })
        };
        let target = CssBox {
            x: number("x")?,
            y: number("y")?,
            width: number("width")?,
            height: number("height")?,
        };
        if [target.x, target.y, target.width, target.height]
            .iter()
            .any(|value| !value.is_finite())
            || target.width <= 0.0
            || target.height <= 0.0
        {
            return Err(DriverError::Protocol {
                capability: "bounding_box".to_owned(),
                message: "response contained invalid target geometry".to_owned(),
            });
        }
        let box_command_sequence = self.records.last().map_or(0, |record| record.sequence);
        let value = path.to_str().ok_or_else(|| {
            DriverError::Artifact("artifact output path is not valid UTF-8".to_owned())
        })?;
        let data = self.execute("screenshot", &["screenshot", value])?;
        let returned =
            data.get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| DriverError::Protocol {
                    capability: "screenshot".to_owned(),
                    message: "response omitted artifact path".to_owned(),
                })?;
        let raw_path = self.artifact(returned, path, MAX_SCREENSHOT_BYTES)?;
        let screenshot_command_sequence = self.records.last().map_or(0, |record| record.sequence);
        if screenshot_command_sequence != box_command_sequence.saturating_add(1) {
            return Err(DriverError::Protocol {
                capability: "capture".to_owned(),
                message: "bounding box and screenshot commands were not adjacent".to_owned(),
            });
        }
        Ok(CaptureBundle {
            raw_path,
            target,
            viewport,
            capture_token: format!(
                "{}:{box_command_sequence}:{screenshot_command_sequence}",
                self.session
            ),
            box_command_sequence,
            screenshot_command_sequence,
        })
    }

    fn diagnostics(&mut self) -> Result<DiagnosticsSummary, DriverError> {
        let console = self.execute("console", &["console"])?;
        let errors = self.execute("page_errors", &["errors"])?;
        let messages = console
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| DriverError::Protocol {
                capability: "console".to_owned(),
                message: "response omitted messages".to_owned(),
            })?;
        let page_errors = errors
            .get("errors")
            .and_then(Value::as_array)
            .ok_or_else(|| DriverError::Protocol {
                capability: "page_errors".to_owned(),
                message: "response omitted errors".to_owned(),
            })?;
        Ok(DiagnosticsSummary {
            console_messages: messages.len(),
            console_sha256: hex_digest(&serde_json::to_vec(messages).unwrap_or_default()),
            page_errors: page_errors.len(),
            page_errors_sha256: hex_digest(&serde_json::to_vec(page_errors).unwrap_or_default()),
        })
    }

    fn stop_trace(&mut self, path: &Path) -> Result<PathBuf, DriverError> {
        if !self.trace_started {
            return Err(DriverError::CommandFailed {
                capability: "trace_stop".to_owned(),
                message: "trace was not started".to_owned(),
            });
        }
        ensure_output_parent(&self.run_root, path)?;
        let value = path.to_str().ok_or_else(|| {
            DriverError::Artifact("trace output path is not valid UTF-8".to_owned())
        })?;
        let data = self.execute("trace_stop", &["trace", "stop", value])?;
        self.trace_started = false;
        let event_count = data
            .get("eventCount")
            .and_then(Value::as_u64)
            .filter(|count| *count > 0)
            .ok_or_else(|| DriverError::Protocol {
                capability: "trace_stop".to_owned(),
                message: "response reported an empty or invalid trace".to_owned(),
            })?;
        let returned =
            data.get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| DriverError::Protocol {
                    capability: "trace_stop".to_owned(),
                    message: "response omitted artifact path".to_owned(),
                })?;
        let artifact = self.artifact(returned, path, MAX_TRACE_BYTES)?;
        let file =
            fs::File::open(&artifact).map_err(|error| DriverError::Artifact(error.to_string()))?;
        let trace: TraceDocument = serde_json::from_reader(file)
            .map_err(|_| DriverError::Artifact("trace is not valid JSON".to_owned()))?;
        if trace.trace_events.is_empty()
            || u64::try_from(trace.trace_events.len()).unwrap_or(u64::MAX) != event_count
        {
            return Err(DriverError::Artifact(
                "trace event count is empty or contradicts the response".to_owned(),
            ));
        }
        Ok(artifact)
    }

    fn close(&mut self) -> Result<(), DriverError> {
        let data = self.execute("close", &["close"])?;
        if data.get("closed").and_then(Value::as_bool) != Some(true) {
            return Err(DriverError::Protocol {
                capability: "close".to_owned(),
                message: "response did not confirm owned session closure".to_owned(),
            });
        }
        Ok(())
    }

    fn records(&self) -> Vec<DriverCommandRecord> {
        self.records.clone()
    }
}

#[derive(Debug, Deserialize)]
struct Envelope {
    success: bool,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TraceDocument {
    #[serde(rename = "traceEvents")]
    trace_events: Vec<IgnoredAny>,
}

struct LimitedRead {
    bytes: Vec<u8>,
    total: usize,
    oversized: bool,
}

fn read_limited(mut reader: impl Read, limit: usize) -> Result<LimitedRead, String> {
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut total = 0usize;
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader.read(&mut chunk).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count);
        let remaining = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..count.min(remaining)]);
    }
    Ok(LimitedRead {
        bytes,
        total,
        oversized: total > limit,
    })
}

fn join_reader(
    handle: thread::JoinHandle<Result<LimitedRead, String>>,
) -> Result<LimitedRead, DriverError> {
    handle
        .join()
        .map_err(|_| DriverError::Io("output reader panicked".to_owned()))?
        .map_err(DriverError::Io)
}

fn add_safe_environment(command: &mut Command, timeout: Duration) {
    const SAFE: &[&str] = &[
        "PATH",
        "HOME",
        "USERPROFILE",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SystemRoot",
        "LOCALAPPDATA",
        "APPDATA",
        "XDG_RUNTIME_DIR",
        "XDG_CACHE_HOME",
    ];
    for key in SAFE {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env(
        "AGENT_BROWSER_DEFAULT_TIMEOUT",
        timeout.as_millis().to_string(),
    );
    // Explicit close remains authoritative. This daemon-side reaper limits
    // orphan lifetime if Crawlson is interrupted before cleanup can run.
    command.env("AGENT_BROWSER_IDLE_TIMEOUT_MS", "60000");
}

fn ensure_output_parent(root: &Path, path: &Path) -> Result<(), DriverError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(DriverError::Io(error.to_string())),
        Ok(_) => {
            return Err(DriverError::Artifact(
                "artifact output path already exists".to_owned(),
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| DriverError::Artifact("artifact path has no parent".to_owned()))?;
    let existing = parent
        .ancestors()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| {
            DriverError::Artifact("artifact path has no existing ancestor".to_owned())
        })?;
    let existing = existing
        .canonicalize()
        .map_err(|error| DriverError::Io(error.to_string()))?;
    if !existing.starts_with(root) {
        return Err(DriverError::Artifact(
            "artifact output escapes the run directory".to_owned(),
        ));
    }
    let parent = parent
        .canonicalize()
        .map_err(|error| DriverError::Io(error.to_string()))?;
    if !parent.starts_with(root) {
        return Err(DriverError::Artifact(
            "artifact output escapes the run directory".to_owned(),
        ));
    }
    Ok(())
}

fn valid_session(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
}

fn validate_extension_browser(path: &Path) -> Result<PathBuf, DriverError> {
    let supplied =
        fs::symlink_metadata(path).map_err(|error| DriverError::Unavailable(error.to_string()))?;
    if supplied.file_type().is_symlink() || !supplied.is_file() {
        return Err(DriverError::Unavailable(
            "browser executable must be a regular non-symlink file".to_owned(),
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| DriverError::Unavailable(error.to_string()))?;
    let mut child = Command::new(&canonical)
        .env_clear()
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| DriverError::Unavailable(error.to_string()))?;
    let status = child
        .wait_timeout(Duration::from_secs(5))
        .map_err(|error| DriverError::Unavailable(error.to_string()))?;
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(DriverError::Unavailable(
            "browser version probe timed out".to_owned(),
        ));
    };
    let mut output = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout
            .take(4096)
            .read_to_end(&mut output)
            .map_err(|error| DriverError::Unavailable(error.to_string()))?;
    }
    if !status.success() {
        return Err(DriverError::Unavailable(
            "browser version probe failed".to_owned(),
        ));
    }
    let version = String::from_utf8_lossy(&output);
    if !version.contains("Chrome for Testing") && !version.contains("Chromium") {
        return Err(DriverError::Unavailable(
            "mutating journeys require Chromium or Chrome for Testing because branded Chrome may ignore unpacked extensions"
                .to_owned(),
        ));
    }
    Ok(canonical)
}

fn navigation_was_blocked(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    (message.contains("domain") || message.contains("navigation"))
        && (message.contains("not allowed")
            || message.contains("not in the allowed domains")
            || message.contains("blocked")
            || message.contains("allowlist"))
}

fn cleanup_grace_capability(capability: &str) -> bool {
    matches!(
        capability,
        "console" | "page_errors" | "trace_stop" | "close"
    )
}

fn command_timeout(
    capability: &str,
    action_timeout: Duration,
    run_deadline: Instant,
    now: Instant,
) -> Option<Duration> {
    if cleanup_grace_capability(capability) {
        Some(action_timeout)
    } else {
        run_deadline
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
            .map(|remaining| remaining.min(action_timeout))
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn validate_click_response(selector: &str, data: &Value) -> Result<(), DriverError> {
    if data.get("clicked").and_then(Value::as_str) == Some(selector) {
        Ok(())
    } else {
        Err(DriverError::Protocol {
            capability: "click".to_owned(),
            message: "response did not acknowledge the requested selector".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_session_contract_is_narrow() {
        assert!(valid_session("crawlson-ab12_cd"));
        assert!(!valid_session("default/session"));
        assert!(!valid_session(&"x".repeat(49)));
    }

    #[test]
    fn read_only_policy_bytes_remain_stable() {
        assert_eq!(
            DriverPolicyMode::ReadOnly.policy(),
            b"{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\"]}\n"
        );
    }

    #[test]
    fn follow_link_policy_adds_only_required_capabilities() {
        let read_only: Value = serde_json::from_slice(DriverPolicyMode::ReadOnly.policy()).unwrap();
        let follow_link: Value =
            serde_json::from_slice(DriverPolicyMode::FollowLink.policy()).unwrap();
        let read_only = read_only["allow"].as_array().unwrap();
        let follow_link = follow_link["allow"].as_array().unwrap();
        let additions = follow_link
            .iter()
            .filter(|capability| !read_only.contains(capability))
            .collect::<Vec<_>>();

        assert_eq!(
            additions,
            vec![
                &Value::String("getattribute".to_owned()),
                &Value::String("isenabled".to_owned()),
                &Value::String("click".to_owned()),
            ]
        );
        assert!(
            read_only
                .iter()
                .all(|capability| follow_link.contains(capability))
        );
    }

    #[test]
    fn authenticated_policies_add_only_state_loading() {
        for (base, authenticated) in [
            (
                DriverPolicyMode::ReadOnly,
                DriverPolicyMode::AuthenticatedReadOnly,
            ),
            (
                DriverPolicyMode::FollowLink,
                DriverPolicyMode::AuthenticatedFollowLink,
            ),
        ] {
            let base: Value = serde_json::from_slice(base.policy()).unwrap();
            let authenticated: Value = serde_json::from_slice(authenticated.policy()).unwrap();
            let base = base["allow"].as_array().unwrap();
            let authenticated = authenticated["allow"].as_array().unwrap();
            let additions = authenticated
                .iter()
                .filter(|capability| !base.contains(capability))
                .collect::<Vec<_>>();

            assert_eq!(additions, vec![&Value::String("state_load".to_owned())]);
            assert!(
                base.iter()
                    .all(|capability| authenticated.contains(capability))
            );
        }
    }

    #[test]
    fn constructor_materializes_the_selected_policy_for_the_owned_run() {
        let run = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let driver = AgentBrowserDriver::new_with_policy_mode(
            &executable,
            run.path(),
            Origin::parse("http://127.0.0.1:4173").unwrap(),
            "crawlson-policy-test".to_owned(),
            Duration::from_secs(5),
            Duration::from_secs(30),
            DriverPolicyMode::FollowLink,
        )
        .unwrap();

        assert_eq!(
            driver.policy_path.file_name().unwrap().to_str().unwrap(),
            "follow-link-policy.json"
        );
        assert_eq!(fs::read(driver.policy_path).unwrap(), FOLLOW_LINK_POLICY);
    }

    #[test]
    fn action_effect_unknown_display_never_exposes_retained_detail() {
        let error = DriverError::ActionEffectUnknown("sensitive upstream detail".to_owned());
        assert_eq!(
            error.to_string(),
            "browser action effect is unknown after dispatch"
        );
    }

    #[test]
    fn click_acknowledgement_must_match_the_dispatched_selector_exactly() {
        assert!(
            validate_click_response("#continue", &serde_json::json!({"clicked": "#continue"}))
                .is_ok()
        );
        for data in [
            serde_json::json!({"clicked": "#other"}),
            serde_json::json!({"clicked": null}),
            serde_json::json!({}),
        ] {
            assert!(matches!(
                validate_click_response("#continue", &data),
                Err(DriverError::Protocol { capability, .. }) if capability == "click"
            ));
        }
    }

    #[test]
    fn bounded_reader_drains_but_retains_only_the_limit() {
        let read = read_limited(&b"123456789"[..], 4).unwrap();
        assert_eq!(read.bytes, b"1234");
        assert_eq!(read.total, 9);
        assert!(read.oversized);
    }

    #[test]
    fn output_validation_never_creates_an_out_of_run_parent() {
        let run = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let parent = outside.path().join("not-created");
        let result = ensure_output_parent(run.path(), &parent.join("artifact.png"));
        assert!(matches!(result, Err(DriverError::Artifact(_))));
        assert!(!parent.exists());
    }

    #[test]
    fn run_deadline_stops_actions_but_preserves_bounded_cleanup_grace() {
        let now = Instant::now();
        let expired = now.checked_sub(Duration::from_secs(1)).unwrap();
        let action = Duration::from_secs(20);
        assert_eq!(command_timeout("text", action, expired, now), None);
        assert_eq!(command_timeout("close", action, expired, now), Some(action));

        let near = now + Duration::from_secs(2);
        assert_eq!(
            command_timeout("navigate", action, near, now),
            Some(Duration::from_secs(2))
        );
    }
}
