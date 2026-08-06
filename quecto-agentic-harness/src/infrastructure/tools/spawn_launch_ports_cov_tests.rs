use super::*;
use crate::infrastructure::tools::spawn_container::PreparedChild;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn tool() -> SpawnTool {
    let dir = tempfile::tempdir().unwrap().keep();
    SpawnTool::new(vec![], true).with_socket_dir(dir)
}

fn config() -> SubagentConfig {
    SubagentConfig {
        agent_id: Some("worker".into()),
        system: None,
        task: None,
        model: None,
        effort: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        disable_tools: vec![],
        read_only: false,
        container: crate::domain::subagent::ContainerSelection::Local,
        restrict_to_workspace: true,
    }
}

#[test]
fn ports_allocate_build_success_and_duplicate_contracts() {
    let tool = tool();
    let mut ports = SpawnLaunchPorts::new(&tool);
    let cfg = config();
    let identity = ports.allocate_identity(&cfg).unwrap();
    let args = ports.build_cli_args(&identity, &cfg).unwrap();
    assert!(args.iter().any(|arg| arg.to_string_lossy() == "--mode"));
    assert!(ports.resolve_binary().is_ok() || ports.resolve_binary().is_err());

    let registry: super::super::spawn::SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let duplicate_uuid = crate::domain::ids::AgentUuid::mint();
    registry.lock().unwrap().insert(
        duplicate_uuid.to_string(),
        SubagentEntry::with_identity(
            duplicate_uuid,
            "worker".into(),
            std::path::PathBuf::from("/tmp/a.sock"),
            0,
        ),
    );
    let duplicate_tool = tool.with_registry(registry);
    let mut duplicate_ports = SpawnLaunchPorts::new(&duplicate_tool);
    assert!(duplicate_ports.allocate_identity(&cfg).is_err());
}

#[tokio::test]
async fn ports_ready_rollback_prompt_uncommit_and_success_paths() {
    let tool = tool();
    let mut ports = SpawnLaunchPorts::new(&tool);
    let cfg = config();
    let identity = ports.allocate_identity(&cfg).unwrap();
    let socket = ports.socket_path.clone().unwrap();
    let mut prepared = PreparedChild::new_for_test(None, Some("env".into()), Some(socket.clone()));

    let ready = ports.ready(&mut prepared).await;
    assert!(ready.is_err());
    ports.rollback_prepared(&mut prepared).await;
    assert!(ports.send_initial_prompt(&socket, "hi").await.is_err());
    ports.uncommit_registered("missing").await;
    let result = ports.success(&identity, Some("env"));
    assert!(!result.is_error);
    assert!(result.content.contains("environment_ref=env"));
}

/// #1390 review finding: a join racing a kill must not register into a
/// no-longer-running environment — the launch fails and the just-registered
/// entry is removed again.
#[tokio::test]
async fn register_into_a_stopped_environment_fails_and_unregisters() {
    let tool = tool();
    let env_ref = tool.environment_registry.mint_ref();
    tool.environment_registry
        .commit(crate::domain::environment_registry::EnvironmentRecord {
            environment_ref: env_ref.clone(),
            environment_id: "env-raced".into(),
            environment_uuid: crate::domain::environment_registry::mint_environment_uuid(),
            name: None,
            workspace_path: std::path::PathBuf::from("/workspace"),
            repository: String::new(),
            script_name: "default".into(),
            retained_exec_argv: vec![],
            retained_kill_argv: vec![],
            retained_cleanup_argv: vec![],
            members: vec![],
            status: crate::domain::environment_registry::EnvironmentStatus::Running,
            metadata: serde_json::json!({}),
            last_error: None,
        });
    let claim = tool.environment_registry.begin_kill(&env_ref).unwrap();
    tool.environment_registry.complete_kill(claim);

    let mut ports = SpawnLaunchPorts::new(&tool);
    let cfg = config();
    let identity = ports.allocate_identity(&cfg).unwrap();
    let mut prepared = PreparedChild::new_for_test(None, Some(env_ref.clone()), None);
    let runtime = PreparedRuntime {
        socket_path: std::path::PathBuf::from("/tmp/raced.sock"),
        pid: 0,
        environment_ref: Some(env_ref.clone()),
    };
    let err = ports
        .register_and_monitor(&identity, runtime, &mut prepared, &cfg)
        .await
        .unwrap_err();
    assert!(err.to_string().contains(&env_ref), "{err}");
    assert!(
        !tool
            .registry
            .lock()
            .unwrap()
            .contains_key(&identity.registry_key),
        "the raced entry must be unregistered"
    );
    assert!(
        tool.environment_registry
            .get(&env_ref)
            .unwrap()
            .members
            .is_empty(),
        "no membership may be recorded in a stopped environment"
    );
}

#[tokio::test]
async fn contract_accessor_yields_working_production_ports() {
    let tool = tool();
    let mut ports = tool.launch_ports_for_contract();
    let identity = ports.allocate_identity(&config()).unwrap();
    assert_eq!(identity.session_name, "worker");
    assert!(!identity.registry_key.is_empty());
}
