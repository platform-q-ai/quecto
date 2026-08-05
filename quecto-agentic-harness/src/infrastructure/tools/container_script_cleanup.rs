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

fn require_string(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing required string field '{field}'"))
}

fn require_metadata(value: &serde_json::Value) -> Result<(), String> {
    value
        .get("metadata")
        .filter(|v| v.is_object())
        .map(|_| ())
        .ok_or_else(|| "missing required object field 'metadata'".to_string())
}

pub(crate) fn validate_inspect_output(stdout: &[u8]) -> Result<(String, String), String> {
    let value: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|e| format!("inspect script did not return JSON: {e}"))?;
    require_string(&value, "environment_id")?;
    let status = require_string(&value, "status")?;
    let health = require_string(&value, "health")?;
    require_string(&value, "workspace_path")?;
    require_string(&value, "container_ref")?;
    require_metadata(&value)?;
    Ok((status, health))
}

pub(crate) fn validate_kill_output(stdout: &[u8]) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|e| format!("kill script did not return JSON: {e}"))?;
    require_string(&value, "environment_id")?;
    require_string(&value, "status")?;
    require_string(&value, "workspace_path")?;
    require_string(&value, "container_ref")?;
    require_metadata(&value)?;
    if !value.get("cleanup").is_some_and(|v| v.is_object())
        && !value
            .get("metadata")
            .and_then(|m| m.get("cleaned"))
            .is_some()
    {
        return Err("kill output missing cleanup result".to_string());
    }
    Ok(())
}

pub(crate) fn run_container_kill_script(entry: &SubagentEntry) -> Result<(), String> {
    let Some(command) = entry.container_kill_command.as_deref() else {
        return Ok(());
    };
    let mut cmd = command_from_config(command)?;
    populate_env(&mut cmd, entry);
    match cmd.output() {
        Ok(out) if out.status.success() => validate_kill_output(&out.stdout),
        Ok(out) => Err(format!(
            "kill script exited with status {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => Err(format!("kill script failed to start: {e}")),
    }
}

pub(super) fn invoke_container_kill_script(entry: &SubagentEntry) -> Result<(), String> {
    run_container_kill_script(entry)
}

pub(crate) fn environment_key(entry: &SubagentEntry) -> Option<&str> {
    entry
        .container_uuid
        .as_deref()
        .or(entry.environment_id.as_deref())
}

pub(crate) fn invoke_container_kill_scripts_once<'a, I>(entries: I) -> Result<(), String>
where
    I: IntoIterator<Item = &'a SubagentEntry>,
{
    let mut cleaned = std::collections::HashSet::new();
    let mut errors = Vec::new();
    for entry in entries {
        let env = environment_key(entry).map(str::to_string);
        if env.as_ref().is_none_or(|e| cleaned.insert(e.clone())) {
            if let Err(err) = invoke_container_kill_script(entry) {
                errors.push(match env {
                    Some(env) => format!("{env}: {err}"),
                    None => err,
                });
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn cleanup_container_environments_after_removal(
    removed: &[(String, SubagentEntry)],
    live: &super::subagent_registry::SubagentRegistry,
    container_registry: Option<&super::container_registry::ContainerRegistry>,
) -> Result<(), String> {
    if let Some(registry) = container_registry {
        for (_, entry) in removed {
            if let (Some(uuid), agent) = (&entry.container_uuid, &entry.agent_uuid) {
                let _ =
                    super::container_registry::remove_agent_from_container(registry, uuid, agent);
            }
        }
    }
    let live_entries = live.lock().unwrap_or_else(|e| e.into_inner());
    let mut cleaned = std::collections::HashSet::new();
    let mut errors = Vec::new();
    for (_, entry) in removed {
        let Some(env) = environment_key(entry) else {
            continue;
        };
        if !cleaned.insert(env.to_string()) {
            continue;
        }
        let has_live_member = if let (Some(registry), Some(uuid)) =
            (container_registry, entry.container_uuid.as_deref())
        {
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .get(uuid)
                .is_some_and(|container| !container.agents.is_empty())
        } else {
            live_entries
                .values()
                .any(|candidate| environment_key(candidate) == Some(env))
        };
        if !has_live_member {
            match invoke_container_kill_script(entry) {
                Ok(()) => {
                    if let (Some(registry), Some(uuid)) =
                        (container_registry, entry.container_uuid.as_deref())
                    {
                        let _ = super::container_registry::set_container_status(
                            registry,
                            uuid,
                            super::container_registry::ContainerStatus::Stopped,
                        );
                    }
                }
                Err(err) => {
                    if let (Some(registry), Some(uuid)) =
                        (container_registry, entry.container_uuid.as_deref())
                    {
                        let _ = super::container_registry::set_container_health(
                            registry,
                            uuid,
                            super::container_registry::ContainerStatus::CleanupFailed,
                            Some(err.clone()),
                        );
                    }
                    errors.push(format!("{env}: {err}"));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn apply_container_inspect(
    registry: &super::subagent_registry::SubagentRegistry,
    container_registry: Option<&super::container_registry::ContainerRegistry>,
    agent_id: &str,
) -> Result<(), String> {
    let entry = {
        let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        entries.get(agent_id).cloned()
    };
    let Some(entry) = entry else { return Ok(()) };
    if entry
        .container_inspect_once
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return Ok(());
    }
    let Some(command) = entry.container_inspect_command.as_deref() else {
        return Ok(());
    };
    let mut cmd =
        command_from_config(command).map_err(|e| format!("inspect command invalid: {e}"))?;
    populate_env(&mut cmd, &entry);
    let output = cmd.output().map_err(|e| {
        persist_container_inspect_failure(
            container_registry,
            &entry,
            format!("inspect script failed to start: {e}"),
        );
        format!("inspect script failed to start: {e}")
    })?;
    if !output.status.success() {
        let err = format!(
            "inspect script exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        persist_container_inspect_failure(container_registry, &entry, err.clone());
        return Err(err);
    }
    let (status, health) = match validate_inspect_output(&output.stdout) {
        Ok(parsed) => parsed,
        Err(err) => {
            persist_container_inspect_failure(container_registry, &entry, err.clone());
            return Err(err);
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
        Ok(value) => value,
        Err(err) => {
            let err = format!("inspect script did not return JSON: {err}");
            persist_container_inspect_failure(container_registry, &entry, err.clone());
            return Err(err);
        }
    };
    let workspace = value
        .get("workspace_path")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    persist_container_inspect_success(container_registry, &entry, &value, &status, &health);
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(current) = entries.get_mut(agent_id) {
        current.environment_health = Some(if health == "healthy" { status } else { health });
        current.last_error = None;
        if let Some(w) = workspace {
            current.workspace_path = Some(w);
        }
    }
    Ok(())
}

fn persist_container_inspect_failure(
    container_registry: Option<&super::container_registry::ContainerRegistry>,
    entry: &SubagentEntry,
    error: String,
) {
    let Some(registry) = container_registry else {
        return;
    };
    let Some(uuid) = entry.container_uuid.as_deref() else {
        return;
    };
    let mut state = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Some(container) = state.entries.get_mut(uuid) else {
        return;
    };
    if matches!(
        container.status,
        super::container_registry::ContainerStatus::Stopped
            | super::container_registry::ContainerStatus::CleanupFailed
    ) {
        return;
    }
    container.status = super::container_registry::ContainerStatus::InspectFailed;
    container.last_error = Some(format!("postmortem inspect failed: {error}"));
}

fn persist_container_inspect_success(
    container_registry: Option<&super::container_registry::ContainerRegistry>,
    entry: &SubagentEntry,
    value: &serde_json::Value,
    status: &str,
    health: &str,
) {
    let Some(registry) = container_registry else {
        return;
    };
    let Some(uuid) = entry.container_uuid.as_deref() else {
        return;
    };
    let mut state = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Some(container) = state.entries.get_mut(uuid) else {
        return;
    };
    if matches!(
        container.status,
        super::container_registry::ContainerStatus::Stopped
            | super::container_registry::ContainerStatus::CleanupFailed
    ) {
        return;
    }
    container.workspace_path = value
        .get("workspace_path")
        .and_then(|v| v.as_str())
        .unwrap_or(&container.workspace_path)
        .to_string();
    container.environment_id = value
        .get("environment_id")
        .and_then(|v| v.as_str())
        .unwrap_or(&container.environment_id)
        .to_string();
    container.metadata = value
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    container.status = if health == "healthy" && status == "running" {
        super::container_registry::ContainerStatus::Running
    } else {
        super::container_registry::ContainerStatus::Unhealthy
    };
    container.last_error = None;
}

pub(crate) fn record_container_health_failure(
    registry: &super::subagent_registry::SubagentRegistry,
    entries_to_mark: impl IntoIterator<Item = String>,
    health: &str,
    error: String,
) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let mut envs = std::collections::HashSet::new();
    for id in entries_to_mark {
        if let Some(entry) = entries.get_mut(&id) {
            entry.environment_health = Some(health.to_string());
            entry.last_error = Some(error.clone());
            if let Some(env) = environment_key(entry) {
                envs.insert(env.to_string());
            }
        }
    }
    for entry in entries.values_mut() {
        if environment_key(entry).is_some_and(|env| envs.contains(env)) {
            entry.environment_health = Some(health.to_string());
            entry.last_error = Some(error.clone());
        }
    }
}
