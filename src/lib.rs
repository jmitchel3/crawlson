pub mod auth;
mod cli;
pub mod collection;
pub mod doctor;
pub mod driver;
pub mod focus;
pub mod install;
pub mod journey;
pub mod net_guard;
pub mod recovery;
pub mod release;
pub mod render;
pub mod runner;
pub mod update;

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_TARGET: &str = env!("CRAWLSON_BUILD_TARGET");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub exit_code: u8,
    pub stdout: String,
    pub stderr: String,
}

impl CommandResult {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            exit_code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn operational_error(stderr: impl Into<String>) -> Self {
        Self {
            exit_code: 1,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    fn write_to_stdio(self) -> ExitCode {
        if !self.stdout.is_empty() {
            let _ = io::stdout().write_all(self.stdout.as_bytes());
        }
        if !self.stderr.is_empty() {
            let _ = io::stderr().write_all(self.stderr.as_bytes());
        }
        ExitCode::from(self.exit_code)
    }
}

pub fn run<I, T>(args: I) -> CommandResult
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    cli::run(args)
}

pub fn main_entry() -> ExitCode {
    run(std::env::args_os()).write_to_stdio()
}
