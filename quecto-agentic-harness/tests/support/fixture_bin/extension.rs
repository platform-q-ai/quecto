//! `extension --socket <path> --agent-id <id> --record <file> [--exit-code <n>]
//! [--tool <name>] [--register-delay <ms>] [--exit-after-register-once <marker>]
//! [--ignore-close]`: a stand-in configured extension (#2446). It appends
//! `<agent_id> <pid> <socket>` to the record file, then either exits at once
//! with `--exit-code` (a crash, a command-line error, refused tools) or
//! connects to the agent's socket, waits `--register-delay`, registers one
//! tool (`--tool`, else `echo_<agent_id>`) and answers each `execute_tool`
//! with its agent id. With `--exit-after-register-once`, the first instance
//! (the marker file absent) creates the marker and exits 1 once its tools
//! are registered. It exits 0 when the agent closes the connection, unless
//! `--ignore-close`, when it lives on until a signal ends it.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Default)]
struct Options {
    socket: String,
    agent_id: String,
    record: String,
    exit_code: Option<i32>,
    tool: Option<String>,
    register_delay: Duration,
    exit_after_register_once: Option<String>,
    ignore_close: bool,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag == "--ignore-close" {
            options.ignore_close = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("extension: {flag} takes a value"))?
            .clone();
        match flag.as_str() {
            "--socket" => options.socket = value,
            "--agent-id" => options.agent_id = value,
            "--record" => options.record = value,
            "--tool" => options.tool = Some(value),
            "--exit-after-register-once" => options.exit_after_register_once = Some(value),
            "--exit-code" => {
                options.exit_code = Some(value.parse().map_err(|_| "extension: --exit-code n")?)
            }
            "--register-delay" => {
                let millis = value
                    .parse()
                    .map_err(|_| "extension: --register-delay ms")?;
                options.register_delay = Duration::from_millis(millis);
            }
            other => return Err(format!("extension: unknown flag {other}")),
        }
    }
    let given = [&options.socket, &options.agent_id, &options.record];
    match given.iter().all(|value| !value.is_empty()) {
        true => Ok(options),
        false => Err("extension: --socket, --agent-id and --record are required".to_owned()),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = options(args)?;
    let mut record = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&options.record)
        .map_err(|error| format!("open {}: {error}", options.record))?;
    writeln!(
        record,
        "{} {} {}",
        options.agent_id,
        std::process::id(),
        options.socket
    )
    .map_err(|error| format!("record: {error}"))?;
    if let Some(code) = options.exit_code {
        std::process::exit(code);
    }
    let stream = UnixStream::connect(&options.socket)
        .map_err(|error| format!("connect {}: {error}", options.socket))?;
    let mut writer = stream.try_clone().map_err(|error| error.to_string())?;
    std::thread::sleep(options.register_delay);
    let tool = options
        .tool
        .clone()
        .unwrap_or_else(|| format!("echo_{}", options.agent_id));
    let register = serde_json::json!({
        "type": "register_tools",
        "id": "register",
        "tools": [{
            "name": tool,
            "description": "Answers with the agent id this instance was launched for",
            "parametersSchema": "{\"type\":\"object\",\"properties\":{}}",
        }],
    });
    writeln!(writer, "{register}").map_err(|error| error.to_string())?;
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if event["type"] == "response" && event["id"] == "register" {
            println!("register_tools: {event}");
            if let Some(marker) = &options.exit_after_register_once
                && std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(marker)
                    .is_ok()
            {
                std::process::exit(1);
            }
        }
        if event["type"] == "execute_tool" {
            let result = serde_json::json!({
                "type": "tool_result",
                "toolCallId": event["toolCallId"],
                "content": options.agent_id,
                "isError": false,
            });
            writeln!(writer, "{result}").map_err(|error| error.to_string())?;
        }
    }
    if options.ignore_close {
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }
    Ok(())
}
