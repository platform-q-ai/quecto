use super::subagent_registry::SubagentEntry;

pub(crate) fn parse_configured_script_command(
    command: &str,
) -> Result<Vec<std::ffi::OsString>, String> {
    if command.trim().is_empty() {
        return Err("empty command".into());
    }
    if command.chars().any(|c| c.is_control() && c != '\t') {
        return Err("control characters are not allowed".into());
    }
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = None;
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, '\'') => quote = Some('\''),
            (None, '"') => quote = Some('"'),
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    args.push(std::ffi::OsString::from(std::mem::take(&mut cur)));
                }
            }
            (_, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                } else {
                    return Err("trailing escape".into());
                }
            }
            (
                None,
                c @ (';' | '|' | '&' | '$' | '`' | '<' | '>' | '*' | '?' | '{' | '}' | '(' | ')'),
            ) => {
                return Err(format!(
                    "shell metacharacter '{c}' is not allowed; configure an executable path plus arguments"
                ));
            }
            (_, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err("unterminated quote".into());
    }
    if !cur.is_empty() {
        args.push(std::ffi::OsString::from(cur));
    }
    if args
        .first()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.starts_with('-'))
    {
        return Err("executable must not start with '-'".into());
    }
    if args.is_empty() {
        Err("empty command".into())
    } else {
        Ok(args)
    }
}

pub(crate) fn command_from_config(command: &str) -> Result<std::process::Command, String> {
    let argv = parse_configured_script_command(command)?;
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    Ok(cmd)
}

fn populate_env(cmd: &mut std::process::Command, entry: &SubagentEntry) {
    if let Some(v) = &entry.container_ref {
        cmd.env("QUECTO_CONTAINER_REF", v);
    }
    if let Some(v) = &entry.container_name {
        cmd.env("QUECTO_CONTAINER_NAME", v);
    }
    if let Some(v) = &entry.container_uuid {
        cmd.env("QUECTO_CONTAINER_UUID", v);
    }
    if let Some(v) = &entry.environment_id {
        cmd.env("QUECTO_ENVIRONMENT_UUID", v);
    }
    if let Some(v) = &entry.workspace_path {
        cmd.env("QUECTO_WORKSPACE_PATH", v);
    }
    if let Some(v) = &entry.container_script_name {
        cmd.env("QUECTO_CONTAINER_SCRIPT", v);
        cmd.env("QUECTO_SCRIPT_NAME", v);
    }
}

pub(crate) fn run_container_kill_script(entry: &SubagentEntry) -> Result<(), String> {
    let Some(command) = entry.container_kill_command.as_deref() else {
        return Ok(());
    };
    let mut cmd = command_from_config(command)?;
    populate_env(&mut cmd, entry);
    match cmd.output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "kill script exited with status {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => Err(format!("kill script failed to start: {e}")),
    }
}

pub(super) fn invoke_container_kill_script(entry: &SubagentEntry) {
    let _ = run_container_kill_script(entry);
}

pub(crate) fn kill_container_owner(entry: &SubagentEntry, removed: &[(String, SubagentEntry)]) {
    let Some(env) = entry
        .environment_id
        .as_deref()
        .or(entry.container_uuid.as_deref())
    else {
        return;
    };
    let remaining = removed
        .iter()
        .filter(|(_, other)| {
            other.agent_uuid != entry.agent_uuid
                && other
                    .environment_id
                    .as_deref()
                    .or(other.container_uuid.as_deref())
                    == Some(env)
        })
        .count();
    if remaining == 0 {
        invoke_container_kill_script(entry);
    }
}

pub(super) fn invoke_container_inspect_script(entry: &SubagentEntry) {
    let Some(command) = entry.container_inspect_command.as_deref() else {
        return;
    };
    let Ok(mut cmd) = command_from_config(command) else {
        return;
    };
    populate_env(&mut cmd, entry);
    let _ = cmd.output();
}
