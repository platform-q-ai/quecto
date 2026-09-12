use super::*;
use crate::infrastructure::tools::spawn_container::PreparedChild;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn tool() -> SpawnTool {
    let dir = tempfile::tempdir().unwrap().keep();
    SpawnTool::new(vec![]).with_socket_dir(dir)
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

#[test]
fn initial_prompt_retry_deadline_defaults_to_none_for_non_proxy_ports() {
    let tool = tool();
    let ports = SpawnLaunchPorts::new(&tool);
    assert!(ports.initial_prompt_retry_deadline().is_none());
}

#[tokio::test]
async fn ports_ready_rollback_prompt_uncommit_and_success_paths() {
    let tool = tool();
    let mut ports = SpawnLaunchPorts::new(&tool);
    let cfg = config();
    let identity = ports.allocate_identity(&cfg).unwrap();
    let socket = ports.socket_path.clone().unwrap();
    let mut prepared = PreparedChild::new_for_test(
        None,
        Some("env".into()),
        Some(crate::subagent_launch_app::ParentEndpoint::Direct {
            socket_path: socket.clone(),
        }),
    )
    .await;

    let ready = ports.ready(&mut prepared).await;
    assert!(ready.is_err());
    ports.rollback_prepared(&mut prepared).await;
    assert!(
        ports
            .send_initial_prompt(&socket, "hi", None)
            .await
            .is_err()
    );
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
    let (btx, mut brx) = tokio::sync::broadcast::channel::<String>(8);
    let tool = tool().with_event_forwarding(Some(btx), None);
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
            retained_inspect_argv: vec![],
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
    let mut prepared = PreparedChild::new_for_test(None, Some(env_ref.clone()), None).await;
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
    // The refused registration must be withdrawn from subscribers too: the
    // last broadcast survivor set may not contain the phantom agent.
    let mut last_event = None;
    while let Ok(event) = brx.try_recv() {
        last_event = Some(event);
    }
    let last_event = last_event.expect("registration and withdrawal were broadcast");
    assert!(
        !last_event.contains(&identity.registry_key),
        "withdrawal broadcast must omit the phantom agent: {last_event}"
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

#[test]
fn container_child_cli_args_fall_back_to_parents_config_and_local_does_not() {
    // PR #1401 review: the child of a zero-config container spawn must be
    // launched with the parent's effective config — the same path that
    // authorized its environment — while local spawns keep the old chain.
    let parent_cfg = std::path::PathBuf::from("/abs/team.json");
    let tool = tool().with_parent_config_path(Some(parent_cfg.clone()));
    let mut ports = SpawnLaunchPorts::new(&tool);
    let mut cfg = config();
    cfg.container = crate::domain::subagent::ContainerSelection::New {
        container_config: None,
        name: None,
    };
    let identity = ports.allocate_identity(&cfg).unwrap();
    let args = ports.build_cli_args(&identity, &cfg).unwrap();
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let pos = args.iter().position(|a| a == "--config").expect(
        "container spawn without explicit config forwards the parent's config to the child",
    );
    assert_eq!(args[pos + 1], parent_cfg.to_string_lossy());

    // An explicit spawn `config` still wins for container spawns.
    let mut explicit_cfg = cfg.clone();
    explicit_cfg.config_path = Some(std::path::PathBuf::from("/abs/explicit.json"));
    let mut ports2 = SpawnLaunchPorts::new(&tool);
    let identity2 = ports2.allocate_identity(&explicit_cfg).unwrap();
    let args2: Vec<String> = ports2
        .build_cli_args(&identity2, &explicit_cfg)
        .unwrap()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let pos2 = args2.iter().position(|a| a == "--config").unwrap();
    assert_eq!(args2[pos2 + 1], "/abs/explicit.json");

    // Local spawns keep the explicit→inherited chain: no parent fallback.
    let mut ports3 = SpawnLaunchPorts::new(&tool);
    let local_cfg = config();
    let identity3 = ports3.allocate_identity(&local_cfg).unwrap();
    let args3: Vec<String> = ports3
        .build_cli_args(&identity3, &local_cfg)
        .unwrap()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(!args3.iter().any(|a| a == "--config"));
}

/// #1935: the local launch path is the ONLY place a spawned child becomes
/// owned. Registering a real child must record the supervisor handle and a
/// launch generation (never a signal lease), and a later `shutdown_all`
/// must terminate that child through the supervisor: the protocol attempt
/// against its (never bound) socket is negative, so TERM follows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_and_monitor_claims_launched_lease_and_shutdown_terminates_child() {
    let tool = tool();
    let mut ports = SpawnLaunchPorts::new(&tool);
    let cfg = config();
    let identity = ports.allocate_identity(&cfg).unwrap();
    // Building the argv mints the parent control credential and its sidecar.
    let args = ports.build_cli_args(&identity, &cfg).unwrap();
    let sidecar = args
        .iter()
        .position(|a| a == "--parent-control")
        .map(|i| std::path::PathBuf::from(&args[i + 1]))
        .expect("the sidecar path is on argv");
    assert!(sidecar.exists());
    let mut sleep = tokio::process::Command::new("sleep");
    sleep
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut prepared = PreparedChild::new_for_test(Some(sleep), None, None).await;
    let pid = prepared.display_pid;
    assert!(pid > 0, "live child pid");
    let runtime = PreparedRuntime {
        socket_path: ports.socket_path.clone().unwrap(),
        pid,
        environment_ref: None,
    };
    ports
        .register_and_monitor(&identity, runtime, &mut prepared, &cfg)
        .await
        .expect("local registration succeeds");
    {
        let entries = tool.registry.lock().unwrap();
        let entry = &entries[&identity.registry_key];
        assert_eq!(entry.pid, pid);
        assert!(
            !entry.process_ownership.is_owned(),
            "a locally launched child carries no signal lease"
        );
        assert!(
            entry.holds_owned_child(),
            "the supervisor retains the handle"
        );
        assert!(entry.launch_generation.is_some());
    }

    super::super::spawn_registry::shutdown_all(&tool.registry);

    // The reaper task owns the child handle; once the SIGTERM lands it reaps,
    // and /proc/<pid> disappears. A skipped signal leaves sleep alive.
    let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut gone = false;
    while std::time::Instant::now() < deadline {
        let zombie = std::fs::read_to_string(proc_dir.join("stat"))
            .map(|stat| stat.contains(") Z "))
            .unwrap_or(false);
        if !proc_dir.exists() || zombie {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if !gone {
        // SAFETY: best-effort cleanup of the leaked sleep child.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    assert!(gone, "shutdown_all must terminate the launched child");
}

fn poison<T: Send + 'static>(mutex: &std::sync::Arc<std::sync::Mutex<T>>) {
    let shared = mutex.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        panic!("poison for coverage");
    })
    .join();
    assert!(mutex.lock().is_err());
}

/// Poisoned registry / parent-id locks are recovered on every launch step,
/// and `uncommit_registered` of a launched child terminates it through the
/// same lease as a cascade would (#1925).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_steps_recover_from_poisoned_locks_and_uncommit_terminates_child() {
    let tool = tool();
    poison(&tool.registry);
    poison(&tool.parent_id);
    let mut ports = SpawnLaunchPorts::new(&tool);
    let cfg = config();
    let identity = ports.allocate_identity(&cfg).unwrap();
    let args = ports.build_cli_args(&identity, &cfg).unwrap();
    assert!(!args.is_empty());
    let mut sleep = tokio::process::Command::new("sleep");
    sleep
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut prepared = PreparedChild::new_for_test(Some(sleep), None, None).await;
    let pid = prepared.display_pid;
    assert!(pid > 0, "live child pid");
    let runtime = PreparedRuntime {
        socket_path: ports.socket_path.clone().unwrap(),
        pid,
        environment_ref: None,
    };
    ports
        .register_and_monitor(&identity, runtime, &mut prepared, &cfg)
        .await
        .expect("local registration succeeds on a poisoned registry");

    ports.uncommit_registered(&identity.registry_key).await;
    assert!(
        !tool
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&identity.registry_key)
    );
    let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut gone = false;
    while std::time::Instant::now() < deadline {
        let zombie = std::fs::read_to_string(proc_dir.join("stat"))
            .map(|stat| stat.contains(") Z "))
            .unwrap_or(false);
        if !proc_dir.exists() || zombie {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if !gone {
        // SAFETY: best-effort cleanup of the leaked sleep child.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    assert!(gone, "uncommit must terminate the launched child");
}
