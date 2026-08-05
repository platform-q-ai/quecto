use super::container_registry::*;

fn entry(id: &str, status: ContainerStatus) -> ContainerEntry {
    ContainerEntry {
        container_uuid: id.into(),
        container_ref: String::new(),
        container_name: None,
        environment_id: id.into(),
        repo_url: None,
        workspace_path: "/workspace".into(),
        status,
        agents: vec![],
        script_name: "dev".into(),
        exec_command: "true".into(),
        inspect_command: "true".into(),
        kill_command: "true".into(),
        socket_path: None,
        socket_proxy: None,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

#[test]
fn refs_are_session_scoped_and_never_reused() {
    let reg = new_container_registry();
    assert_eq!(
        register_container(&reg, entry("one", ContainerStatus::Stopped)).container_ref,
        "C1"
    );
    assert_eq!(
        register_container(&reg, entry("two", ContainerStatus::Running)).container_ref,
        "C2"
    );
    assert_eq!(
        register_container(&reg, entry("one", ContainerStatus::Running)).container_ref,
        "C1"
    );
    let listed = list_containers(&reg);
    assert_eq!(
        listed
            .iter()
            .map(|e| e.container_ref.as_str())
            .collect::<Vec<_>>(),
        vec!["C1", "C2"]
    );
    assert_eq!(listed[0].status, ContainerStatus::Stopped);
}

#[test]
fn list_containers_sorts_unparseable_refs_last() {
    let reg = new_container_registry();
    register_container(&reg, entry("one", ContainerStatus::Running));
    {
        let mut state = reg.lock().unwrap();
        let mut weird = entry("weird", ContainerStatus::Unhealthy);
        weird.container_ref = "custom".into();
        state.entries.insert(weird.container_uuid.clone(), weird);
    }
    let refs: Vec<_> = list_containers(&reg)
        .into_iter()
        .map(|e| e.container_ref)
        .collect();
    assert_eq!(refs, vec!["C1", "custom"]);
}

#[test]
fn stale_or_unknown_refs_error_without_guessing() {
    let reg = new_container_registry();
    register_container(&reg, entry("one", ContainerStatus::Stopped));
    assert!(
        resolve_live_ref(&reg, "C1")
            .unwrap_err()
            .contains("not live")
    );
    assert!(
        resolve_live_ref(&reg, "C2")
            .unwrap_err()
            .contains("unknown")
    );
}

#[test]
fn existing_container_membership_can_roll_back_without_poisoning_live_ref() {
    let reg = new_container_registry();
    register_container(&reg, entry("one", ContainerStatus::Running));
    add_agent_to_live_container(&reg, "one", crate::domain::ids::AgentUuid::new("agent-1"))
        .unwrap();
    remove_agent_from_container(&reg, "one", &crate::domain::ids::AgentUuid::new("agent-1"))
        .unwrap();

    let listed = list_containers(&reg);
    assert_eq!(
        listed[0].agents,
        Vec::<crate::domain::ids::AgentUuid>::new()
    );
    assert_eq!(resolve_live_ref(&reg, "C1").unwrap(), "one");
}

#[test]
fn existing_container_membership_rollback_is_idempotent() {
    let reg = new_container_registry();
    register_container(&reg, entry("one", ContainerStatus::Running));
    remove_agent_from_container(&reg, "one", &crate::domain::ids::AgentUuid::new("missing"))
        .unwrap();
    assert_eq!(resolve_live_ref(&reg, "C1").unwrap(), "one");
}

#[test]
fn registry_operations_recover_from_poisoned_mutex() {
    let reg = new_container_registry();
    let poisoned = reg.clone();
    let _ = std::panic::catch_unwind(move || {
        let _guard = poisoned.lock().unwrap();
        panic!("poison registry");
    });

    let registered = register_container(&reg, entry("one", ContainerStatus::Running));
    assert_eq!(registered.container_ref, "C1");
    assert_eq!(resolve_live_ref(&reg, "C1").unwrap(), "one");
    assert_eq!(list_containers(&reg).len(), 1);
}

#[test]
fn registry_types_support_diagnostics_and_value_semantics() {
    let original = entry("diag", ContainerStatus::Unhealthy);
    let cloned = original.clone();
    assert_eq!(original, cloned);
    assert!(format!("{original:?}").contains("diag"));
    assert_eq!(format!("{:?}", ContainerStatus::Running), "Running");
    assert_eq!(
        format!("{:?}", ContainerRegistryState::default()),
        "ContainerRegistryState { next_ref: 0, entries: {}, refs: {} }"
    );
}
