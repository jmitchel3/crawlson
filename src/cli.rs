use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::doctor::{self, DoctorOptions};
use crate::render::{self, RenderOptions};
use crate::runner::{self, RunOptions};
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

    /// Run one validated, explicitly authorized read-only journey.
    Run(RunArgs),

    /// Render findings or a guide from one completed, verified run.
    Render(RenderArgs),

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

#[derive(Debug, Args)]
struct RunArgs {
    /// TOML journey definition to validate and run.
    #[arg(value_name = "JOURNEY")]
    journey: PathBuf,

    /// Exact HTTP(S) origin authorized for this run.
    #[arg(long, value_name = "ORIGIN")]
    allow_origin: Option<String>,

    /// Parent directory beneath which a unique run directory is created.
    #[arg(long, value_name = "DIRECTORY", default_value = "crawlson-runs")]
    output_dir: PathBuf,

    /// Exact agent-browser executable to use instead of searching PATH.
    #[arg(long, value_name = "PATH")]
    agent_browser: Option<PathBuf>,

    /// Per-command driver deadline; must remain below agent-browser's IPC limit.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 20,
        value_parser = clap::value_parser!(u64).range(1..=29)
    )]
    action_timeout_seconds: u64,

    /// Overall execution deadline before bounded evidence cleanup begins.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(30..=3600)
    )]
    run_timeout_seconds: u64,
}

#[derive(Debug, Args)]
struct RenderArgs {
    /// Completed Crawlson run directory containing report.json and evidence.
    #[arg(value_name = "RUN_DIRECTORY")]
    run_directory: PathBuf,

    /// Exact journey source used by the completed run.
    #[arg(long, value_name = "JOURNEY", required = true)]
    journey: PathBuf,
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
        Commands::Run(args) => {
            let report = runner::run(RunOptions {
                journey_path: args.journey,
                allowed_origin: args.allow_origin,
                output_directory: args.output_dir,
                agent_browser: args.agent_browser,
                action_timeout: std::time::Duration::from_secs(args.action_timeout_seconds),
                run_timeout: std::time::Duration::from_secs(args.run_timeout_seconds),
            });
            let mut rendered = report.render(cli.json);
            update::finish_foreground(&mut rendered, !cli.json);
            rendered
        }
        Commands::Render(args) => {
            let report = render::run(RenderOptions {
                run_directory: args.run_directory,
                journey_path: args.journey,
            });
            let mut rendered = report.render(cli.json);
            update::finish_foreground(&mut rendered, !cli.json);
            rendered
        }
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
