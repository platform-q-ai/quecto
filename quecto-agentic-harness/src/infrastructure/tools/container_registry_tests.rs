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
        metadata: serde_json::json!({}),
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
