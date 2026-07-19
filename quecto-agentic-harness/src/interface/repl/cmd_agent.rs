// /agent command handler for the REPL.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;

use super::parsers::{is_valid_agent_name, parse_agent_args};
use super::{CMD_AGENT, ReplLoop};

impl<R: BufRead, W: Write> ReplLoop<R, W> {
    pub(super) fn handle_agent(&mut self, input: &str, rt: &tokio::runtime::Runtime) {
        let rest = input.strip_prefix(CMD_AGENT).unwrap_or("").trim();
        let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
        let subcmd = parts.first().copied().unwrap_or("");
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        match subcmd {
            "list" => self.agent_list(),
            "create" => self.agent_create(args_str),
            "show" => self.agent_show(args_str),
            "edit" => self.agent_edit(args_str),
            "remove" => self.agent_remove(args_str),
            "run" => self.agent_run(args_str, rt),
            _ => self.agent_usage(),
        }
    }

    pub(super) fn agents_dir(&self) -> PathBuf {
        self.session.base_dir.join("agents")
    }

    /// Validate and return the path to an agent profile. Returns an error
    /// message if the name is empty or contains path traversal characters.
    pub(super) fn validated_agent_path(&mut self, name: &str) -> Option<PathBuf> {
        if name.is_empty() {
            let _ = writeln!(self.writer, "Error: missing agent name");
            return None;
        }
        if !is_valid_agent_name(name) {
            let _ = writeln!(self.writer, "Error: invalid agent name '{}'", name);
            return None;
        }
        Some(self.agents_dir().join(format!("{}.json", name)))
    }

    fn agent_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /agent <subcommand>");
        let _ = writeln!(self.writer, "  list     List all subagent profiles");
        let _ = writeln!(self.writer, "  create   Create a new profile");
        let _ = writeln!(self.writer, "  show     Show a profile's configuration");
        let _ = writeln!(self.writer, "  edit     Edit an existing profile");
        let _ = writeln!(self.writer, "  remove   Remove a profile");
        let _ = writeln!(self.writer, "  run      Run a task using a profile");
    }

    fn agent_list(&mut self) {
        let dir = self.agents_dir();
        if !dir.exists() {
            let _ = writeln!(self.writer, "No subagent profiles configured");
            return;
        }

        let entries: Vec<String> = std::fs::read_dir(&dir)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                    .filter_map(|e| {
                        e.path()
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        if entries.is_empty() {
            let _ = writeln!(self.writer, "No subagent profiles configured");
            return;
        }

        let _ = writeln!(self.writer, "Subagent profiles:");
        for name in &entries {
            let _ = writeln!(self.writer, "  {}", name);
        }
    }

    fn agent_create(&mut self, args_str: &str) {
        match parse_agent_args(args_str) {
            Ok(parsed) => {
                if parsed.system.is_none() {
                    let _ = writeln!(self.writer, "Error: missing required flag: --system");
                    return;
                }

                if !is_valid_agent_name(&parsed.name) {
                    let _ = writeln!(self.writer, "Error: invalid agent name '{}'", parsed.name);
                    return;
                }

                let dir = self.agents_dir();
                let path = dir.join(format!("{}.json", parsed.name));

                if path.exists() {
                    let _ = writeln!(self.writer, "Error: agent '{}' already exists", parsed.name);
                    return;
                }

                if let Err(e) = std::fs::create_dir_all(&dir) {
                    let _ = writeln!(self.writer, "Error: {}", e);
                    return;
                }

                let mut profile = serde_json::json!({
                    "name": parsed.name,
                    "system": parsed.system
                });
                if let Some(ref model) = parsed.model {
                    profile["model"] = serde_json::json!(model);
                }

                match serde_json::to_string_pretty(&profile) {
                    Ok(content) => match std::fs::write(&path, content) {
                        Ok(()) => {
                            let _ = writeln!(self.writer, "Agent '{}' created", parsed.name);
                        }
                        Err(e) => {
                            let _ = writeln!(self.writer, "Error: {}", e);
                        }
                    },
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
            }
        }
    }

    fn agent_show(&mut self, args_str: &str) {
        let name = args_str.trim();
        let Some(path) = self.validated_agent_path(name) else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if let Ok(profile) = serde_json::from_str::<serde_json::Value>(&content) {
                    let _ = writeln!(self.writer, "Agent: {}", name);
                    if let Some(system) = profile["system"].as_str() {
                        let _ = writeln!(self.writer, "System: {}", system);
                    }
                    if let Some(model) = profile["model"].as_str() {
                        let _ = writeln!(self.writer, "Model: {}", model);
                    }
                } else {
                    let _ = writeln!(self.writer, "Error: invalid profile for '{}'", name);
                }
            }
            Err(_) => {
                let _ = writeln!(self.writer, "Error: agent '{}' not found", name);
            }
        }
    }

    fn agent_edit(&mut self, args_str: &str) {
        match parse_agent_args(args_str) {
            Ok(parsed) => {
                let Some(path) = self.validated_agent_path(&parsed.name) else {
                    return;
                };
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let mut profile: serde_json::Value =
                            serde_json::from_str(&content).unwrap_or_default();
                        if let Some(ref system) = parsed.system {
                            profile["system"] = serde_json::json!(system);
                        }
                        if let Some(ref model) = parsed.model {
                            profile["model"] = serde_json::json!(model);
                        }
                        match serde_json::to_string_pretty(&profile) {
                            Ok(updated) => match std::fs::write(&path, updated) {
                                Ok(()) => {
                                    let _ =
                                        writeln!(self.writer, "Agent '{}' updated", parsed.name);
                                }
                                Err(e) => {
                                    let _ = writeln!(self.writer, "Error: {}", e);
                                }
                            },
                            Err(e) => {
                                let _ = writeln!(self.writer, "Error: {}", e);
                            }
                        }
                    }
                    Err(_) => {
                        let _ = writeln!(self.writer, "Error: agent '{}' not found", parsed.name);
                    }
                }
            }
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
            }
        }
    }

    fn agent_remove(&mut self, args_str: &str) {
        let name = args_str.trim();
        let Some(path) = self.validated_agent_path(name) else {
            return;
        };
        if !path.exists() {
            let _ = writeln!(self.writer, "Error: agent '{}' not found", name);
            return;
        }

        match std::fs::remove_file(&path) {
            Ok(()) => {
                let _ = writeln!(self.writer, "Agent '{}' removed", name);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn agent_run(&mut self, args_str: &str, rt: &tokio::runtime::Runtime) {
        let parts: Vec<&str> = args_str.splitn(2, char::is_whitespace).collect();
        let name = parts.first().copied().unwrap_or("").trim();
        let task = if parts.len() > 1 { parts[1].trim() } else { "" };

        let Some(path) = self.validated_agent_path(name) else {
            return;
        };

        let profile = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(p) => p,
                Err(_) => {
                    let _ = writeln!(self.writer, "Error: invalid profile for '{}'", name);
                    return;
                }
            },
            Err(_) => {
                let _ = writeln!(self.writer, "Error: agent '{}' not found", name);
                return;
            }
        };

        if task.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let system = profile["system"].as_str().unwrap_or("").to_string();

        let mut run_messages = Vec::new();

        if !system.is_empty() {
            run_messages.push(Message::system(system));
        }

        run_messages.push(Message::user(task.to_string()));

        let result = rt.block_on(self.session.agent.process(&mut run_messages));

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
}

#[cfg(test)]
#[path = "cmd_agent_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cmd_agent_cov_tests.rs"]
mod cov_tests;
