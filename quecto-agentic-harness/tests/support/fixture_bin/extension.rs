//! `extension --socket <path> --agent-id <id> --record <file> [--exit-code <n>]`:
//! a stand-in configured extension (#2446). It appends `<agent_id> <pid>
//! <socket>` to the record file, then either exits at once with
//! `--exit-code` (a crash, a command-line error, refused tools) or connects
//! to the agent's socket, registers one tool, `echo_<agent_id>`, answers
//! each `execute_tool` with its agent id, and exits 0 when the agent closes
//! the connection.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

struct Options {
    socket: String,
    agent_id: String,
    record: String,
    exit_code: Option<i32>,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut socket = None;
    let mut agent_id = None;
    let mut record = None;
    let mut exit_code = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("extension: {flag} takes a value"))?;
        match flag.as_str() {
            "--socket" => socket = Some(value.clone()),
            "--agent-id" => agent_id = Some(value.clone()),
            "--record" => record = Some(value.clone()),
            "--exit-code" => {
                exit_code = Some(value.parse().map_err(|_| "extension: --exit-code n")?)
            }
            other => return Err(format!("extension: unknown flag {other}")),
        }
    }
    Ok(Options {
        socket: socket.ok_or("extension: --socket")?,
        agent_id: agent_id.ok_or("extension: --agent-id")?,
        record: record.ok_or("extension: --record")?,
        exit_code,
    })
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
    let register = serde_json::json!({
        "type": "register_tools",
        "id": "register",
        "tools": [{
            "name": format!("echo_{}", options.agent_id),
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
    Ok(())
}
