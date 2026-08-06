use std::sync::Arc;

use super::*;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};
use crate::environment_control_app::EnvironmentControlUseCase;

fn use_case(registry: EnvironmentRegistry) -> Arc<EnvironmentControlUseCase> {
    let kill_port = Arc::new(super::super::environment_kill::ScriptEnvironmentKill::new(
        super::super::subagent_registry::new_registry(),
        None,
    ));
    Arc::new(EnvironmentControlUseCase::new(registry, kill_port))
}

fn committed_registry() -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref,
        environment_id: "env-tool".into(),
        environment_uuid: mint_environment_uuid(),
        name: Some("tool-env".into()),
        workspace_path: std::path::PathBuf::from("/ws/tool"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".into()],
        retained_cleanup_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    registry
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn container_commands_are_recognized() {
    assert!(is_container_command(
        &serde_json::json!({"command":"get_containers"})
    ));
    assert!(is_container_command(
        &serde_json::json!({"command":"kill_container"})
    ));
    assert!(!is_container_command(
        &serde_json::json!({"command":"kill"})
    ));
}

#[test]
fn container_commands_require_wiring_and_star_agent_id() {
    let missing = block_on(execute_container_command(
        None,
        &serde_json::json!({"agent_id":"*","command":"get_containers"}),
    ));
    assert!(missing.is_error && missing.content.contains("not available"));

    let uc = use_case(EnvironmentRegistry::new());
    let wrong_target = block_on(execute_container_command(
        Some(&uc),
        &serde_json::json!({"agent_id":"child","command":"get_containers"}),
    ));
    assert!(wrong_target.is_error && wrong_target.content.contains("agent_id '*'"));
}

#[test]
fn kill_container_decodes_exactly_one_string_target() {
    let uc = use_case(committed_registry());
    for args in [
        serde_json::json!({"agent_id":"*","command":"kill_container"}),
        serde_json::json!({"agent_id":"*","command":"kill_container","ref":"C1","name":"x"}),
        serde_json::json!({"agent_id":"*","command":"kill_container","ref":1}),
    ] {
        let result = block_on(execute_container_command(Some(&uc), &args));
        assert!(result.is_error, "{}", result.content);
    }
}

#[test]
fn listing_and_kill_round_trip_through_the_use_case() {
    let uc = use_case(committed_registry());
    let listing = block_on(execute_container_command(
        Some(&uc),
        &serde_json::json!({"agent_id":"*","command":"get_containers"}),
    ));
    assert!(!listing.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&listing.content).unwrap();
    assert_eq!(parsed["containers"][0]["ref"], "C1");
    assert_eq!(parsed["containers"][0]["status"], "empty");

    let killed = block_on(execute_container_command(
        Some(&uc),
        &serde_json::json!({"agent_id":"*","command":"kill_container","name":"tool-env"}),
    ));
    assert!(!killed.is_error, "{}", killed.content);

    let unknown = block_on(execute_container_command(
        Some(&uc),
        &serde_json::json!({"agent_id":"*","command":"kill_container","ref":"C9"}),
    ));
    assert!(unknown.is_error && unknown.content.contains("unknown"));
}

#[test]
fn use_case_debug_is_redacted_but_present() {
    let uc = use_case(EnvironmentRegistry::new());
    assert!(format!("{uc:?}").contains("EnvironmentControlUseCase"));
}
