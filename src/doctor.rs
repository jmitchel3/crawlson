use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use semver::{Version, VersionReq};
use serde::Serialize;
use wait_timeout::ChildExt;

use crate::{CommandResult, VERSION};

const AGENT_BROWSER_REQUIREMENT: &str = ">=0.26.0, <0.27.0";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct DoctorOptions {
    pub executable: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub schema_version: u8,
    pub crawlson_version: &'static str,
    pub status: CheckStatus,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_version: Option<Version>,
    pub supported_range: &'static str,
}

impl DoctorReport {
    pub fn render(&self, json: bool) -> CommandResult {
        let exit_code = if self.status == CheckStatus::Pass {
            0
        } else {
            1
        };
        if json {
            let mut stdout = serde_json::to_string(self).expect("doctor report is serializable");
            stdout.push('\n');
            CommandResult {
                exit_code,
                stdout,
                stderr: String::new(),
            }
        } else {
            let mut stdout = format!("Crawlson doctor ({VERSION})\n");
            for check in &self.checks {
                let marker = if check.status == CheckStatus::Pass {
                    "ok"
                } else {
                    "failed"
                };
                stdout.push_str(&format!("[{marker}] {}: {}\n", check.name, check.message));
            }
            CommandResult {
                exit_code,
                stdout,
                stderr: String::new(),
            }
        }
    }
}

pub fn run(options: DoctorOptions) -> DoctorReport {
    let executable = options.executable.or_else(find_agent_browser);
    let check = match executable {
        Some(path) => probe_agent_browser(&path),
        None => DoctorCheck {
            name: "agent-browser",
            status: CheckStatus::Fail,
            message: format!(
                "not found; install a version matching {AGENT_BROWSER_REQUIREMENT} and ensure it is on PATH"
            ),
            executable: None,
            detected_version: None,
            supported_range: AGENT_BROWSER_REQUIREMENT,
        },
    };

    DoctorReport {
        schema_version: 1,
        crawlson_version: VERSION,
        status: check.status,
        checks: vec![check],
    }
}

fn probe_agent_browser(path: &Path) -> DoctorCheck {
    let base = DoctorCheck {
        name: "agent-browser",
        status: CheckStatus::Fail,
        message: String::new(),
        executable: Some(path.to_path_buf()),
        detected_version: None,
        supported_range: AGENT_BROWSER_REQUIREMENT,
    };

    let mut child = match Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return DoctorCheck {
                message: format!("could not execute: {error}"),
                ..base
            };
        }
    };

    match child.wait_timeout(PROBE_TIMEOUT) {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return DoctorCheck {
                message: format!("version probe exceeded {} seconds", PROBE_TIMEOUT.as_secs()),
                ..base
            };
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return DoctorCheck {
                message: format!("could not wait for version probe: {error}"),
                ..base
            };
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            return DoctorCheck {
                message: format!("could not read version output: {error}"),
                ..base
            };
        }
    };

    if !output.status.success() {
        return DoctorCheck {
            message: format!("version probe exited with {}", output.status),
            ..base
        };
    }

    let stdout = match std::str::from_utf8(&output.stdout) {
        Ok(stdout) if stdout.len() <= 4096 => stdout,
        Ok(_) => {
            return DoctorCheck {
                message: "version output exceeded 4096 bytes".to_owned(),
                ..base
            };
        }
        Err(_) => {
            return DoctorCheck {
                message: "version output was not UTF-8".to_owned(),
                ..base
            };
        }
    };

    let version = match parse_agent_browser_version(stdout) {
        Ok(version) => version,
        Err(message) => return DoctorCheck { message, ..base },
    };
    let requirement = VersionReq::parse(AGENT_BROWSER_REQUIREMENT)
        .expect("the agent-browser requirement is valid semver");
    if !requirement.matches(&version) || !version.pre.is_empty() {
        return DoctorCheck {
            message: format!("found {version}, but Crawlson requires {AGENT_BROWSER_REQUIREMENT}"),
            detected_version: Some(version),
            ..base
        };
    }

    DoctorCheck {
        status: CheckStatus::Pass,
        message: format!("found supported version {version}"),
        detected_version: Some(version),
        ..base
    }
}

fn parse_agent_browser_version(output: &str) -> Result<Version, String> {
    let line = output.trim();
    let value = line
        .strip_prefix("agent-browser ")
        .ok_or_else(|| "unexpected version output; expected 'agent-browser <semver>'".to_owned())?;
    if value.contains(char::is_whitespace) {
        return Err("unexpected trailing data in version output".to_owned());
    }
    Version::parse(value).map_err(|error| format!("invalid agent-browser version: {error}"))
}

fn find_agent_browser() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os("CRAWLSON_AGENT_BROWSER") {
        return executable_candidate(PathBuf::from(explicit));
    }

    let path = env::var_os("PATH")?;
    let names = executable_names("agent-browser");
    env::split_paths(&path)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find_map(executable_candidate)
}

fn executable_candidate(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

fn executable_names(name: &str) -> Vec<OsString> {
    #[cfg(windows)]
    {
        let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".EXE;.CMD;.BAT".into());
        let mut names = vec![OsString::from(name)];
        names.extend(
            extensions
                .to_string_lossy()
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(|extension| format!("{name}{extension}"))
                .map(OsString::from),
        );
        names
    }
    #[cfg(not(windows))]
    {
        vec![OsString::from(name)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_supported_version_shape_strictly() {
        assert_eq!(
            parse_agent_browser_version("agent-browser 0.26.0\n").unwrap(),
            Version::new(0, 26, 0)
        );
        assert!(parse_agent_browser_version("0.26.0").is_err());
        assert!(parse_agent_browser_version("agent-browser 0.26.0 extra").is_err());
    }
}
