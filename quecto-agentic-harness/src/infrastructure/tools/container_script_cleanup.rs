use super::subagent_registry::SubagentEntry;

fn populate_env(cmd: &mut std::process::Command, entry: &SubagentEntry) {
    if let Some(container_ref) = &entry.container_ref {
        cmd.env("QUECTO_CONTAINER_REF", container_ref);
    }
    if let Some(container_uuid) = &entry.container_uuid {
        cmd.env("QUECTO_CONTAINER_UUID", container_uuid);
    }
    if let Some(environment_id) = &entry.environment_id {
        cmd.env("QUECTO_ENVIRONMENT_UUID", environment_id);
    }
    if let Some(workspace_path) = &entry.workspace_path {
        cmd.env("QUECTO_WORKSPACE_PATH", workspace_path);
    }
}

pub(super) fn invoke_container_kill_script(entry: &SubagentEntry) {
    let Some(command) = entry.container_kill_command.as_deref() else {
        return;
    };
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    populate_env(&mut cmd, entry);
    let _ = cmd.output();
}

pub(super) fn invoke_container_inspect_script(entry: &SubagentEntry) {
    let Some(command) = entry.container_inspect_command.as_deref() else {
        return;
    };
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    populate_env(&mut cmd, entry);
    let _ = cmd.output();
}
