use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments == ["--version"] {
        version_probe();
        return;
    }

    let fixture = fixture_directory();
    log_arguments(&fixture, &arguments);
    validate_environment();
    validate_globals(&arguments);
    let scenario = fs::read_to_string(fixture.join("scenario"))
        .unwrap_or_else(|_| "pass".to_owned())
        .trim()
        .to_owned();
    let command = command_arguments(&arguments);
    match command.as_slice() {
        [name, operation, width, height]
            if name == "set"
                && operation == "viewport"
                && width == "1280"
                && height == "720" =>
        {
            if scenario == "prepare_timeout" {
                std::thread::sleep(Duration::from_secs(2));
            }
            if scenario == "prepare_error" {
                failure("fixture prepare failed", 1);
            }
            success(r#"{"width":1280,"height":720,"deviceScaleFactor":1.0,"mobile":false}"#);
        }
        [name, operation] if name == "trace" && operation == "start" => {
            success(r#"{"started":true}"#);
        }
        [name, operation, path] if name == "trace" && operation == "stop" => {
            if scenario == "trace_stop_error" {
                failure("fixture trace stop failed", 1);
            }
            let returned = if scenario == "trace_path_escape" {
                fixture.join("escaped-trace.json")
            } else {
                PathBuf::from(path)
            };
            let contents: &[u8] = match scenario.as_str() {
                "trace_malformed" => b"not-json",
                "trace_empty" => b"{\"traceEvents\":[]}",
                _ => b"{\"traceEvents\":[{\"name\":\"fixture\"}]}",
            };
            fs::write(&returned, contents).unwrap();
            let count = match scenario.as_str() {
                "trace_zero" => 0,
                "trace_mismatch" => 2,
                _ => 1,
            };
            success(&format!(
                r#"{{"path":"{}","eventCount":{count}}}"#,
                json(returned.to_string_lossy().as_ref())
            ));
        }
        [name, url] if name == "open" => {
            if scenario == "domain_block" {
                failure("Domain 'other.example' is not in the allowed domains list", 1);
            }
            let observed = if scenario == "redirect" {
                "http://127.0.0.1:9999/escaped"
            } else if scenario == "credential_current_url" {
                "http://user:secret@127.0.0.1:9999/private/path?token=hidden"
            } else {
                url
            };
            fs::write(fixture.join("current-url"), observed).unwrap();
            if scenario == "escaped_open_response" {
                success(r#"{"title":"Fixture","url":"http://127.0.0.1:9999/escaped"}"#);
                return;
            }
            let response_url = if scenario == "credential_current_url" {
                url
            } else {
                observed
            };
            success(&format!(
                r#"{{"title":"Fixture","url":"{}"}}"#,
                json(response_url)
            ));
        }
        [name, what] if name == "get" && what == "url" => {
            let url = fs::read_to_string(fixture.join("current-url"))
                .unwrap_or_else(|_| "about:blank".to_owned());
            success(&format!(r#"{{"url":"{}"}}"#, json(url.trim())));
        }
        [name, what, _selector] if name == "get" && what == "text" => match scenario.as_str() {
            "malformed" => println!("not-json"),
            "oversized" => println!("{}", "x".repeat(1_048_577)),
            "oversized_stderr" => {
                eprintln!("{}", "x".repeat(65_537));
                success(r#"{"origin":"http://127.0.0.1:4173","text":"Hello"}"#);
            }
            "timeout" => {
                std::thread::sleep(Duration::from_secs(2));
                success(r#"{"origin":"http://127.0.0.1:4173","text":"Hello"}"#);
            }
            "command_error" => failure("fixture text lookup failed", 1),
            "success_with_error" => {
                println!(
                    "{}",
                    r#"{"success":true,"data":{"text":"Hello"},"error":"contradiction"}"#
                )
            }
            "failure_with_data" => {
                println!(
                    "{}",
                    r#"{"success":false,"data":{"text":"Hello"},"error":"failed"}"#
                );
                std::process::exit(1);
            }
            "failure_without_error" => {
                println!("{}", r#"{"success":false,"data":null,"error":null}"#);
                std::process::exit(1);
            }
            "exit_success_envelope_failure" => {
                println!(
                    "{}",
                    r#"{"success":false,"data":null,"error":"failed"}"#
                )
            }
            "exit_failure_envelope_success" => {
                println!(
                    "{}",
                    r#"{"success":true,"data":{"origin":"http://127.0.0.1:4173","text":"Hello"},"error":null}"#
                );
                std::process::exit(1);
            }
            "confirmation_required" => success(
                r#"{"confirmation_required":true,"origin":"http://127.0.0.1:4173","text":"Hello"}"#,
            ),
            "success_missing_data" => {
                println!("{}", r#"{"success":true,"data":null,"error":null}"#)
            }
            "escaped_text_origin" => {
                success(r#"{"origin":"http://127.0.0.1:9999/escaped","text":"Hello"}"#)
            }
            "fail_text" => {
                success(r#"{"origin":"http://127.0.0.1:4173","text":"Different"}"#)
            }
            _ => success(r#"{"origin":"http://127.0.0.1:4173","text":"Hello"}"#),
        },
        [name, state, _selector] if name == "is" && state == "visible" => {
            success(if scenario == "escaped_visible_origin" {
                r#"{"visible":true,"origin":"http://127.0.0.1:9999/escaped"}"#
            } else if scenario == "hidden" {
                r#"{"visible":false,"origin":"http://127.0.0.1:4173/"}"#
            } else {
                r#"{"visible":true,"origin":"http://127.0.0.1:4173/"}"#
            });
        }
        [name, what, _selector] if name == "get" && what == "box" => {
            success(if scenario == "invalid_box" {
                r#"{"x":100.0,"y":100.0,"width":0.0,"height":50.0}"#
            } else {
                r#"{"x":100.0,"y":100.0,"width":200.0,"height":50.0}"#
            });
        }
        [name, path] if name == "screenshot" => {
            fs::copy(fixture.join("screenshot.png"), path).unwrap();
            success(&format!(r#"{{"path":"{}"}}"#, json(path)));
        }
        [name] if name == "console" => {
            if scenario == "diagnostics_error" {
                failure("fixture diagnostics failed", 1);
            }
            success(r#"{"messages":[]}"#);
        }
        [name] if name == "errors" => {
            if scenario == "page_errors_error" {
                failure("fixture page errors failed", 1);
            }
            success(r#"{"errors":[]}"#);
        }
        [name] if name == "close" => {
            if scenario == "cleanup_fail" {
                failure("fixture cleanup failed", 1);
            } else {
                success(r#"{"closed":true}"#);
            }
        }
        _ => failure(&format!("unexpected arguments: {command:?}"), 9),
    }
}

fn version_probe() {
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

fn fixture_directory() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn log_arguments(directory: &Path, arguments: &[String]) {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("calls.log"))
        .unwrap();
    writeln!(log, "{}", arguments.join("\t")).unwrap();
}

fn command_arguments(arguments: &[String]) -> Vec<String> {
    arguments.get(17..).unwrap_or_default().to_vec()
}

fn validate_environment() {
    if std::env::var_os("CRAWLSON_UNSAFE_TEST_ENV").is_some() {
        failure("unsafe environment reached the driver", 9);
    }
    if std::env::var("AGENT_BROWSER_IDLE_TIMEOUT_MS").as_deref() != Ok("60000") {
        failure("idle timeout was not configured", 9);
    }
    let timeout = std::env::var("AGENT_BROWSER_DEFAULT_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    if !timeout.is_some_and(|value| (1..30_000).contains(&value)) {
        failure("action timeout was not configured safely", 9);
    }
}

fn validate_globals(arguments: &[String]) {
    let valid_shape = arguments.len() >= 18
        && arguments[0] == "--session"
        && arguments[1].starts_with("crawlson-")
        && arguments[2] == "--json"
        && arguments[3] == "--config"
        && arguments[5] == "--allowed-domains"
        && arguments[6] == "127.0.0.1"
        && arguments[7] == "--action-policy"
        && arguments[9] == "--content-boundaries"
        && arguments[10] == "--max-output"
        && arguments[11] == "65536"
        && arguments[12] == "--headed"
        && arguments[13] == "false"
        && arguments[14] == "--no-auto-dialog"
        && arguments[15] == "--screenshot-format"
        && arguments[16] == "png";
    if !valid_shape {
        failure("invalid global argument contract", 9);
    }
    let config = fs::read_to_string(&arguments[4]).unwrap_or_default();
    if config != "{\"headed\":false,\"noAutoDialog\":true,\"screenshotFormat\":\"png\"}\n" {
        failure("invalid owned configuration", 9);
    }
    let policy = fs::read_to_string(&arguments[8]).unwrap_or_default();
    let expected = "{\"default\":\"deny\",\"allow\":[\"launch\",\"viewport\",\"trace_start\",\"trace_stop\",\"navigate\",\"url\",\"gettext\",\"isvisible\",\"boundingbox\",\"screenshot\",\"console\",\"errors\",\"close\"]}\n";
    if policy != expected {
        failure("invalid action policy", 9);
    }
}

fn success(data: &str) {
    println!(r#"{{"success":true,"data":{data},"error":null}}"#);
}

fn failure(message: &str, code: i32) -> ! {
    println!(
        r#"{{"success":false,"data":null,"error":"{}"}}"#,
        json(message)
    );
    std::process::exit(code);
}

fn json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
