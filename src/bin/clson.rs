use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match canonical_binary() {
        Ok(binary) => match Command::new(binary)
            .args(std::env::args_os().skip(1))
            .status()
        {
            Ok(status) => exit_code(status),
            Err(error) => {
                eprintln!("clson: could not start the sibling crawlson binary: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("clson: {error}");
            ExitCode::FAILURE
        }
    }
}

fn canonical_binary() -> Result<PathBuf, String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("could not locate this executable: {error}"))?;
    let directory = current
        .parent()
        .ok_or_else(|| "this executable has no parent directory".to_owned())?;
    let mut name = PathBuf::from("crawlson");
    if let Some(extension) = current.extension() {
        name.set_extension(extension);
    }
    Ok(directory.join(name))
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from)
}
