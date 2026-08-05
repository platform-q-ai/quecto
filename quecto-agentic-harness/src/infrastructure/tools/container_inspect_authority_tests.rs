use std::io::Write;

use super::container_registry::*;
use super::container_script_cleanup::apply_container_inspect;
use super::subagent_registry::{SubagentEntry, new_registry};

fn container_entry(uuid: &str) -> ContainerEntry {
    ContainerEntry {
        container_uuid: uuid.into(),
        container_ref: String::new(),
        container_name: Some(format!("{uuid}-name")),
        environment_id: uuid.into(),
        repo_url: Some("https://example.invalid/repo.git".into()),
        workspace_path: "/workspace/original".into(),
        status: ContainerStatus::Running,
        agents: vec![crate::domain::ids::AgentUuid::new("child")],
        script_name: "dev".into(),
        exec_command: "true".into(),
        inspect_command: "true".into(),
        kill_command: "true".into(),
        socket_path: None,
        socket_proxy: None,
        metadata: serde_json::json!({"before": true}),
        last_error: None,
    }
}

fn script(contents: &str) -> tempfile::TempPath {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.as_file().metadata().unwrap().permissions();
        perms.set_mode(0o755);
        file.as_file().set_permissions(perms).unwrap();
    }
    file.into_temp_path()
}

fn subagent_with_inspect(uuid: &str, inspect_command: String) -> SubagentEntry {
    let mut entry = SubagentEntry::new("/tmp/child.sock".into(), 42);
    entry.container_uuid = Some(uuid.into());
    entry.container_ref = Some("C1".into());
    entry.environment_id = Some(uuid.into());
    entry.workspace_path = Some("/workspace/original".into());
    entry.container_script_name = Some("dev".into());
    entry.container_kill_command = Some("true".into());
    entry.container_inspect_command = Some(inspect_command);
    entry
}

#[test]
fn failed_postmortem_inspect_survives_subagent_removal_in_container_registry() {
    let containers = new_container_registry();
    register_container(&containers, container_entry("env-fail"));
    let inspect = script("#!/usr/bin/env bash\necho inspect boom >&2\nexit 7\n");
    let subagents = new_registry();
    subagents.lock().unwrap().insert(
        "child".into(),
        subagent_with_inspect("env-fail", inspect.display().to_string()),
    );

    let err = apply_container_inspect(&subagents, Some(&containers), "child").unwrap_err();
    assert!(err.contains("inspect boom"));
    subagents.lock().unwrap().remove("child");
    containers
        .lock()
        .unwrap()
        .entries
        .get_mut("env-fail")
        .unwrap()
        .agents
        .clear();

    let listed = list_containers(&containers);
    assert!(listed[0].agents.is_empty());
    assert_eq!(listed[0].status, ContainerStatus::InspectFailed);
    assert!(
        listed[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains("inspect boom")
    );
}

#[test]
fn successful_postmortem_inspect_updates_authoritative_metadata() {
    let containers = new_container_registry();
    register_container(&containers, container_entry("env-ok"));
    let inspect = script(
        "#!/usr/bin/env bash\nprintf '%s\\n' '{\"environment_id\":\"env-ok\",\"status\":\"running\",\"health\":\"healthy\",\"workspace_path\":\"/workspace/new\",\"container_ref\":\"C1\",\"metadata\":{\"fresh\":true}}'\n",
    );
    let subagents = new_registry();
    subagents.lock().unwrap().insert(
        "child".into(),
        subagent_with_inspect("env-ok", inspect.display().to_string()),
    );

    apply_container_inspect(&subagents, Some(&containers), "child").unwrap();
    subagents.lock().unwrap().clear();

    let current = resolve_container_ref_any(&containers, "env-ok").unwrap();
    assert_eq!(current.status, ContainerStatus::Running);
    assert_eq!(current.workspace_path, "/workspace/new");
    assert_eq!(current.metadata, serde_json::json!({"fresh": true}));
    assert_eq!(current.last_error, None);
}

#[test]
fn repeated_eof_inspect_is_exactly_once_even_after_failure() {
    let containers = new_container_registry();
    register_container(&containers, container_entry("env-once"));
    let dir = tempfile::tempdir().unwrap();
    let count = dir.path().join("count");
    let inspect = script(&format!(
        "#!/usr/bin/env bash\necho hit >> {}\necho boom >&2\nexit 2\n",
        count.display()
    ));
    let subagents = new_registry();
    subagents.lock().unwrap().insert(
        "child".into(),
        subagent_with_inspect("env-once", inspect.display().to_string()),
    );

    assert!(apply_container_inspect(&subagents, Some(&containers), "child").is_err());
    apply_container_inspect(&subagents, Some(&containers), "child").unwrap();

    assert_eq!(std::fs::read_to_string(count).unwrap().lines().count(), 1);
    assert_eq!(
        resolve_container_ref_any(&containers, "env-once")
            .unwrap()
            .status,
        ContainerStatus::InspectFailed
    );
}

#[test]
fn stopped_state_takes_precedence_over_late_inspect_failure() {
    let containers = new_container_registry();
    register_container(&containers, container_entry("env-stopped"));
    mark_container_stopped(&containers, "env-stopped").unwrap();
    let inspect = script("#!/usr/bin/env bash\necho too late >&2\nexit 9\n");
    let subagents = new_registry();
    subagents.lock().unwrap().insert(
        "child".into(),
        subagent_with_inspect("env-stopped", inspect.display().to_string()),
    );

    assert!(apply_container_inspect(&subagents, Some(&containers), "child").is_err());

    let current = resolve_container_ref_any(&containers, "env-stopped").unwrap();
    assert_eq!(current.status, ContainerStatus::Stopped);
    assert_eq!(current.last_error, None);
}
