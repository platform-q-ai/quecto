// /spawn command handler for the REPL.

use std::io::{BufRead, Write};

use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;

use super::parsers::parse_spawn_args;
use super::{CMD_SPAWN, ReplLoop};

impl<R: BufRead, W: Write> ReplLoop<R, W> {
    pub(super) fn handle_spawn(&mut self, input: &str, rt: &tokio::runtime::Runtime) {
        let rest = input.strip_prefix(CMD_SPAWN).unwrap_or("").trim();

        if rest.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let parsed = match parse_spawn_args(rest) {
            Ok(p) => p,
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
                return;
            }
        };

        if parsed.help {
            self.spawn_usage();
            return;
        }

        if parsed.task.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        if parsed.model.is_some() {
            let _ = writeln!(
                self.writer,
                "Error: --model is not supported in REPL mode (agent uses the model from startup)"
            );
            return;
        }

        // Resolve system prompt: from --agent profile or --system flag
        let system = if let Some(ref agent_name) = parsed.agent {
            let Some(path) = self.validated_agent_path(agent_name) else {
                return;
            };
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(profile) => profile["system"].as_str().map(String::from),
                    Err(_) => {
                        let _ =
                            writeln!(self.writer, "Error: invalid profile for '{}'", agent_name);
                        return;
                    }
                },
                Err(_) => {
                    let _ = writeln!(self.writer, "Error: agent '{}' not found", agent_name);
                    return;
                }
            }
        } else {
            parsed.system.clone()
        };

        // Build an ephemeral message list for the spawn (session isolation).
        let mut spawn_messages = Vec::new();

        if let Some(ref prompt) = system {
            spawn_messages.push(Message::system(prompt.clone()));
        }

        spawn_messages.push(Message::user(parsed.task.clone()));

        // Run with optional timeout
        let result = if let Some(max_secs) = parsed.max_time {
            let timeout = std::time::Duration::from_secs(max_secs);
            rt.block_on(async {
                match tokio::time::timeout(timeout, self.session.agent.process(&mut spawn_messages))
                    .await
                {
                    Ok(r) => r,
                    Err(_) => Err(crate::domain::error::DomainError::Other(
                        "spawn timed out".to_string(),
                    )),
                }
            })
        } else {
            rt.block_on(self.session.agent.process(&mut spawn_messages))
        };

        match result {
            Ok(r) => {
                self.session
                    .messages
                    .push(Message::assistant(r.response.clone(), vec![]));
                let _ = writeln!(self.writer, "{}", r.response);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {e}");
            }
        }
    }

    pub(super) fn spawn_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /spawn [flags] <task>");
        let _ = writeln!(
            self.writer,
            "  --agent <name>       Use a named agent profile"
        );
        let _ = writeln!(self.writer, "  --system <prompt>    Set a system prompt");
        let _ = writeln!(
            self.writer,
            "  --model <model>      Not supported in REPL mode"
        );
        let _ = writeln!(
            self.writer,
            "  --max-time <secs>    Set a timeout in seconds"
        );
        let _ = writeln!(self.writer, "  --help               Show this help");
    }
}

#[cfg(test)]
#[path = "cmd_spawn_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "cmd_spawn_tests.rs"]
mod tests;
