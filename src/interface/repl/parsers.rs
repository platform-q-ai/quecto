// Argument parsers and shell tokenizer for REPL slash commands.

use crate::domain::cron::CronSchedule;

/// Parsed arguments for `/cron add`.
#[derive(Debug)]
pub(super) struct ParsedCronAdd {
    pub name: String,
    pub message: String,
    pub schedule: CronSchedule,
    pub deliver_to: Option<String>,
}

/// Parse `/cron add <name> --interval N --message ... [--deliver-to ...] [--cron ...]`
///
/// Uses simple token-based parsing that handles single-quoted values.
pub(super) fn parse_cron_add_args(args_str: &str) -> Result<ParsedCronAdd, String> {
    let tokens = shell_split_repl(args_str);
    if tokens.is_empty() {
        return Err("missing job name".to_string());
    }

    let name = tokens[0].clone();
    let mut message: Option<String> = None;
    let mut interval: Option<u64> = None;
    let mut cron_expr: Option<String> = None;
    let mut deliver_to: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--interval" => {
                if i + 1 < tokens.len() {
                    interval = Some(
                        tokens[i + 1]
                            .parse::<u64>()
                            .map_err(|_| "invalid interval value".to_string())?,
                    );
                    i += 2;
                } else {
                    return Err("--interval requires a value".to_string());
                }
            }
            "--cron" => {
                if i + 1 < tokens.len() {
                    cron_expr = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--cron requires a value".to_string());
                }
            }
            "--message" => {
                if i + 1 < tokens.len() {
                    // Collect all remaining tokens that aren't flags as the message
                    let mut msg_parts = Vec::new();
                    i += 1;
                    while i < tokens.len() && !tokens[i].starts_with("--") {
                        msg_parts.push(tokens[i].clone());
                        i += 1;
                    }
                    if msg_parts.is_empty() {
                        return Err("--message requires a value".to_string());
                    }
                    message = Some(msg_parts.join(" "));
                } else {
                    return Err("--message requires a value".to_string());
                }
            }
            "--deliver-to" => {
                if i + 1 < tokens.len() {
                    deliver_to = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--deliver-to requires a value".to_string());
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    let message = message.ok_or_else(|| "missing required flag: --message".to_string())?;
    let schedule = match (interval, cron_expr) {
        (_, Some(expression)) => CronSchedule::Cron { expression },
        (Some(seconds), None) => {
            if seconds == 0 {
                return Err("interval must be greater than 0".to_string());
            }
            CronSchedule::Interval { seconds }
        }
        (None, None) => {
            return Err("missing schedule: specify --interval or --cron".to_string());
        }
    };

    Ok(ParsedCronAdd {
        name,
        message,
        schedule,
        deliver_to,
    })
}

/// Simple shell-like token splitter for REPL command arguments.
///
/// Handles single-quoted and double-quoted strings. Does not handle
/// backslash escapes (sufficient for REPL slash command parsing).
/// Uses `chars()` iteration to correctly handle multi-byte UTF-8.
pub(super) fn shell_split_repl(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch == ' ' {
            chars.next();
            continue;
        }
        let mut current = String::new();
        if ch == '\'' || ch == '"' {
            let quote = ch;
            chars.next();
            while let Some(&c) = chars.peek() {
                if c == quote {
                    chars.next();
                    break;
                }
                current.push(c);
                chars.next();
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\'' || c == '"' {
                    break;
                }
                current.push(c);
                chars.next();
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    tokens
}

/// Parsed arguments for `/agent create` or `/agent edit`.
#[derive(Debug)]
pub(super) struct ParsedAgentArgs {
    pub name: String,
    pub system: Option<String>,
    pub model: Option<String>,
}

/// Parse `/agent create|edit <name> [--system ...] [--model ...]`
///
/// The name is the first token. `--system` collects all subsequent tokens
/// until the next `--` flag (or end). `--model` takes a single token.
pub(super) fn parse_agent_args(args_str: &str) -> Result<ParsedAgentArgs, String> {
    let tokens = shell_split_repl(args_str);
    if tokens.is_empty() {
        return Err("missing agent name".to_string());
    }

    let name = tokens[0].clone();
    let mut system: Option<String> = None;
    let mut model: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--system" => {
                if i + 1 < tokens.len() {
                    let mut parts = Vec::new();
                    i += 1;
                    while i < tokens.len() && !tokens[i].starts_with("--") {
                        parts.push(tokens[i].clone());
                        i += 1;
                    }
                    if parts.is_empty() {
                        return Err("--system requires a value".to_string());
                    }
                    system = Some(parts.join(" "));
                } else {
                    return Err("--system requires a value".to_string());
                }
            }
            "--model" => {
                if i + 1 < tokens.len() {
                    model = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--model requires a value".to_string());
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(ParsedAgentArgs {
        name,
        system,
        model,
    })
}

/// Check whether an agent name is valid (safe for use as a filename).
///
/// Allowed: ASCII alphanumeric, hyphens, underscores. 1-64 characters.
pub(super) fn is_valid_agent_name(name: &str) -> bool {
    let len = name.len();
    if len == 0 || len > 64 {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

/// Parsed arguments for `/spawn`.
#[derive(Debug)]
pub(super) struct ParsedSpawnArgs {
    pub agent: Option<String>,
    pub system: Option<String>,
    pub model: Option<String>,
    pub max_time: Option<u64>,
    pub task: String,
    pub help: bool,
}

/// Parse `/spawn [--agent name] [--system prompt] [--model model] [--max-time secs] [--help] <task>`
pub(super) fn parse_spawn_args(args_str: &str) -> Result<ParsedSpawnArgs, String> {
    let tokens = shell_split_repl(args_str);
    if tokens.is_empty() {
        return Ok(ParsedSpawnArgs {
            agent: None,
            system: None,
            model: None,
            max_time: None,
            task: String::new(),
            help: false,
        });
    }

    let mut agent: Option<String> = None;
    let mut system: Option<String> = None;
    let mut model: Option<String> = None;
    let mut max_time: Option<u64> = None;
    let mut help = false;
    let mut task_parts = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--help" => {
                help = true;
                i += 1;
            }
            "--agent" => {
                if i + 1 < tokens.len() {
                    agent = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--agent requires a value".to_string());
                }
            }
            "--system" => {
                if i + 1 < tokens.len() {
                    // Take the next token as the system prompt. If the user
                    // wants a multi-word prompt they must quote it:
                    //   /spawn --system 'You are a translator' task
                    system = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--system requires a value".to_string());
                }
            }
            "--model" => {
                if i + 1 < tokens.len() {
                    model = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--model requires a value".to_string());
                }
            }
            "--max-time" => {
                if i + 1 < tokens.len() {
                    max_time = Some(
                        tokens[i + 1]
                            .parse::<u64>()
                            .map_err(|_| "invalid --max-time value".to_string())?,
                    );
                    i += 2;
                } else {
                    return Err("--max-time requires a value".to_string());
                }
            }
            _ => {
                // Everything else is the task
                task_parts.push(tokens[i].clone());
                i += 1;
            }
        }
    }

    Ok(ParsedSpawnArgs {
        agent,
        system,
        model,
        max_time,
        task: task_parts.join(" "),
        help,
    })
}

#[cfg(test)]
#[path = "parsers_tests.rs"]
mod tests;
