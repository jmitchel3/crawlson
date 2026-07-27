use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::doctor::{self, DoctorOptions};
use crate::update::{self, ManualUpgradeOptions};
use crate::{CommandResult, VERSION};

#[derive(Debug, Parser)]
#[command(
    name = "crawlson",
    version = VERSION,
    about = "Agent-driven browser journeys with reproducible evidence",
    arg_required_else_help = true
)]
struct Cli {
    /// Emit one machine-readable JSON object on stdout.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print Crawlson version information.
    Version,

    /// Check Crawlson and agent-browser availability without changing them.
    Doctor(DoctorArgs),

    /// Check for or install a Crawlson release.
    Upgrade(UpgradeArgs),

    /// Internal isolated worker for periodic update checks.
    #[command(name = "__update-worker", hide = true)]
    UpdateWorker,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    /// Exact agent-browser executable to probe instead of searching PATH.
    #[arg(long, value_name = "PATH")]
    agent_browser: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct UpgradeArgs {
    /// Report update availability without changing the installation.
    #[arg(long)]
    check: bool,

    /// Perform no network access.
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Serialize)]
struct VersionReport<'a> {
    schema_version: u8,
    name: &'a str,
    version: &'a str,
    target: &'a str,
}

pub(crate) fn run<I, T>(args: I) -> CommandResult
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(2);
            let message = error.to_string();
            return if exit_code == 0 {
                CommandResult::success(message)
            } else {
                CommandResult {
                    exit_code,
                    stdout: String::new(),
                    stderr: message,
                }
            };
        }
    };

    match cli.command {
        Commands::Version => version(cli.json),
        Commands::Doctor(args) => {
            let result = doctor::run(DoctorOptions {
                executable: args.agent_browser,
            });
            let mut rendered = result.render(cli.json);
            update::finish_foreground(&mut rendered, !cli.json);
            rendered
        }
        Commands::Upgrade(args) => update::run_manual(ManualUpgradeOptions {
            check_only: args.check,
            offline: args.offline,
            json: cli.json,
        }),
        Commands::UpdateWorker => update::run_periodic_worker(),
    }
}

fn version(json: bool) -> CommandResult {
    if json {
        let report = VersionReport {
            schema_version: 1,
            name: "crawlson",
            version: VERSION,
            target: crate::BUILD_TARGET,
        };
        let mut output = serde_json::to_string(&report).expect("version report is serializable");
        output.push('\n');
        let mut result = CommandResult::success(output);
        update::finish_foreground(&mut result, false);
        result
    } else {
        let mut result = CommandResult::success(format!("crawlson {VERSION}\n"));
        update::finish_foreground(&mut result, true);
        result
    }
}
