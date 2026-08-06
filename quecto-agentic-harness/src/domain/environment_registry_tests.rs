use super::*;

fn record(env_ref: &str, id: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: id.to_string(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

#[test]
fn refs_are_monotonic_never_reused_and_scoped_per_registry() {
    let registry = EnvironmentRegistry::new();
    let first = registry.mint_ref();
    let second = registry.mint_ref();
    assert_eq!(first, "C1");
    assert_eq!(second, "C2");

    // A failed launch consumes its ref: removal never recycles it.
    registry.commit(record(&second, "env-a"));
    registry.remove(&second);
    assert_eq!(registry.mint_ref(), "C3");

    // Registries are session-scoped, not process-global.
    let other_session = EnvironmentRegistry::new();
    assert_eq!(other_session.mint_ref(), "C1");
}

#[test]
fn commit_get_remove_round_trip() {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(record(&env_ref, "env-a"));
    assert_eq!(registry.get(&env_ref).unwrap().environment_id, "env-a");
    assert_eq!(registry.entries().len(), 1);

    assert_eq!(registry.remove(&env_ref).unwrap().environment_id, "env-a");
    assert!(registry.get(&env_ref).is_none());
    assert!(registry.entries().is_empty());
    assert!(registry.remove(&env_ref).is_none());
}

#[test]
fn lock_poison_recovery_keeps_registry_usable() {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1", "env-a"));

    let poisoner = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.state.lock().unwrap();
        panic!("poison the environment registry lock");
    })
    .join();

    // Every accessor must recover from the poisoned lock without losing state.
    assert_eq!(registry.mint_ref(), "C1");
    registry.commit(record("C2", "env-b"));
    assert_eq!(registry.get("C1").unwrap().environment_id, "env-a");
    assert_eq!(registry.entries().len(), 2);
    assert_eq!(registry.remove("C2").unwrap().environment_id, "env-b");
}
