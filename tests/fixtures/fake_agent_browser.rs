use std::time::Duration;

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments != ["--version"] {
        eprintln!("unexpected arguments: {arguments:?}");
        std::process::exit(9);
    }

    if let Ok(milliseconds) = std::env::var("FAKE_AGENT_BROWSER_DELAY_MS") {
        let milliseconds = milliseconds.parse().expect("delay is an integer");
        std::thread::sleep(Duration::from_millis(milliseconds));
    }
    if let Ok(code) = std::env::var("FAKE_AGENT_BROWSER_EXIT") {
        std::process::exit(code.parse().expect("exit code is an integer"));
    }

    println!(
        "{}",
        std::env::var("FAKE_AGENT_BROWSER_OUTPUT")
            .unwrap_or_else(|_| "agent-browser 0.26.0".to_owned())
    );
}
