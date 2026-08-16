use super::super::subagent_registry::SubagentStatus;
use super::*;

fn test_tool() -> SpawnTool {
    SpawnTool::new(
        vec!["news-bot".to_string(), "weather-bot".to_string()],
        true,
    )
}
#[test]
fn parse_args_accepts_by_value_workflow_spec() {
    let tool = test_tool();
    let args = r#"{"task":"t","workflow_spec":{"template":{"id":"rev","label":"Rev","description":"d","steps":[{"key":"a","label":"A","phase":"review"}]}}}"#;
    let cfg = tool.parse_args_for_test(args).expect("should parse");
    let spec = cfg.workflow_spec.expect("workflow_spec should be set");
    assert_eq!(spec.template.id, "rev");
    assert_eq!(spec.template.steps.len(), 1);
}

#[tokio::test]
async fn workflow_spec_seeds_binding_before_first_monitor_event() {
    let tool = SpawnTool::new(vec![], true);
    tool.execute(r#"{"task":"t","agent_id":"bound","workflow_spec":{"template":{"id":"rev","label":"Rev","description":"d","steps":[{"key":"a","label":"A","phase":"review"},{"key":"b","label":"B","phase":"review"}]}}}"#)
        .await
        .expect("stub spawn must succeed");
    let registry = tool.registry.lock().unwrap();
    let workflow = registry
        .values()
        .find(|entry| entry.display_name == "bound")
        .and_then(|entry| entry.workflow.as_ref())
        .expect("bound workflow metadata must exist at registration");
    assert_eq!(workflow.mode, "active");
    assert_eq!(workflow.steps_completed, 0);
    assert_eq!(workflow.steps_total, 2);
}

#[test]
fn parse_args_rejects_workflow_spec_without_template() {
    let tool = test_tool();
    let args = r#"{"task":"t","workflow_spec":{"inputs":{"pr":7}}}"#;
    let err = tool.parse_args_for_test(args).unwrap_err();
    assert!(err.contains("invalid workflow_spec"), "got: {err}");
}

#[test]
fn parse_args_without_workflow_spec_leaves_it_none() {
    let tool = test_tool();
    let cfg = tool.parse_args_for_test(r#"{"task":"t"}"#).unwrap();
    assert!(cfg.workflow_spec.is_none());
}

#[test]
fn parse_read_only_marks_config_as_observer() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"review","read_only":true}"#)
        .unwrap();
    assert!(
        cfg.read_only,
        "read_only spawn arguments must mark the sub-agent as an observer"
    );
}

#[test]
fn parse_disable_write_and_edit_marks_config_as_observer() {
    let tool = SpawnTool::new(vec![], true);
    for args in [
        r#"{"task":"review","disable_tools":["write","edit"]}"#,
        r#"{"task":"review","disable_tools":["edit","write"]}"#,
        r#"{"task":"review","disable_tools":["read","write","edit"]}"#,
    ] {
        let cfg = tool.parse_args(args).unwrap();
        assert!(
            cfg.read_only,
            "disabling both write and edit must mark the sub-agent as read-only for {args}"
        );
    }
}

#[test]
fn parse_single_mutation_tool_disabled_does_not_mark_config_as_observer() {
    let tool = SpawnTool::new(vec![], true);
    for args in [
        r#"{"task":"review","disable_tools":["write"]}"#,
        r#"{"task":"review","disable_tools":["edit"]}"#,
    ] {
        let cfg = tool.parse_args(args).unwrap();
        assert!(
            !cfg.read_only,
            "disabling only one mutation tool must not mark the sub-agent as read-only for {args}"
        );
    }
}
#[tokio::test]
async fn execute_stub_mode_registers_read_only_observer() {
    let tool = SpawnTool::new(vec![], true);
    let _result = tool
        .execute(r#"{"task":"review","agent_id":"reviewer","read_only":true}"#)
        .await
        .unwrap();
    let registry = tool.registry.lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "reviewer")
        .expect("spawned read-only sub-agent should be registered");
    assert!(
        entry.read_only,
        "registered sub-agent state must identify read-only observers"
    );
}

#[test]
fn write_private_new_creates_private_file_and_replaces_stale() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("wf.json");
    write_private_new(&path, b"first").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
    // A stale file (O_EXCL hits AlreadyExists) is removed and recreated once.
    write_private_new(&path, b"second").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "spec file must be owner-only");
    }
}

#[test]
fn test_definition() {
    let tool = test_tool();
    let def = tool.definition();
    assert_eq!(def.name, "spawn");
    assert!(!def.description.is_empty());
    assert!(def.description.contains("agent_cmd"));
    assert!(def.description.contains("get_messages"));
    assert!(!def.description.contains("get_messages_tail"));
}

#[test]
fn test_definition_task_not_required() {
    let tool = test_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    // No "required" array — task is optional
    assert!(
        schema.get("required").is_none(),
        "task should not be required in schema"
    );
}

#[test]
fn test_parse_valid_task() {
    let tool = test_tool();
    let config = tool.parse_args(r#"{"task":"Summarize news"}"#).unwrap();
    assert_eq!(config.task.as_deref(), Some("Summarize news"));
    assert!(config.agent_id.is_none());
}

#[test]
fn test_parse_without_task() {
    let tool = test_tool();
    let config = tool.parse_args(r#"{"agent_id":"news-bot"}"#).unwrap();
    assert!(config.task.is_none());
    assert_eq!(config.agent_id.as_deref(), Some("news-bot"));
}

#[test]
fn test_parse_empty_object() {
    let tool = test_tool();
    let config = tool.parse_args(r#"{}"#).unwrap();
    assert!(config.task.is_none());
    assert!(config.agent_id.is_none());
}

#[test]
fn test_parse_with_agent_id() {
    let tool = test_tool();
    let config = tool
        .parse_args(r#"{"task":"Get weather","agent_id":"weather-bot"}"#)
        .unwrap();
    assert_eq!(config.agent_id.as_deref(), Some("weather-bot"));
}

#[test]
fn test_parse_disallowed_agent() {
    let tool = test_tool();
    let result = tool.parse_args(r#"{"task":"Evil task","agent_id":"evil-bot"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not allowed"));
}

#[test]
fn test_parse_empty_allowlist_permits_any() {
    let tool = SpawnTool::new(vec![], true);
    let config = tool
        .parse_args(r#"{"task":"Do stuff","agent_id":"any-bot"}"#)
        .unwrap();
    assert_eq!(config.agent_id.as_deref(), Some("any-bot"));
}

#[test]
fn test_parse_with_system_prompt() {
    let tool = test_tool();
    let config = tool
        .parse_args(r#"{"task":"Summarize","system":"You are a summarizer"}"#)
        .unwrap();
    assert_eq!(config.system.as_deref(), Some("You are a summarizer"));
}

#[test]
fn test_parse_rejects_invalid_agent_id_format() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.parse_args(r#"{"task":"Do stuff","agent_id":"../escape"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
}

#[test]
fn test_with_base_dir_sets_fields() {
    let base = PathBuf::from("/tmp/quecto-test");
    let tool = SpawnTool::with_base_dir(vec!["bot-a".to_string()], false, base.clone());
    assert_eq!(tool.base_dir, base);
    assert_eq!(tool.allowed_agents, vec!["bot-a".to_string()]);
    assert!(!tool.restrict_to_workspace);
}

#[test]
fn test_new_sets_empty_base_dir() {
    let tool = SpawnTool::new(vec![], false);
    assert!(tool.base_dir.as_os_str().is_empty());
}

#[test]
fn test_with_registry_shares_state() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let tool = SpawnTool::new(vec![], true).with_registry(registry.clone());
    registry.lock().unwrap().insert(
        "test".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 123),
    );
    assert!(
        tool.registry
            .lock()
            .unwrap()
            .values()
            .any(|entry| entry.display_name == "test")
    );
}
#[test]
fn register_and_broadcast_emits_immediate_state_changed() {
    // #866: spawn registration must broadcast the survivor set at once so a child
    // that begins a long first turn is visible in the TUI immediately, without
    // waiting for a GetSubagents poll or a terminal event.
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let entry = SubagentEntry::new(PathBuf::from("/tmp/x.sock"), 0);
    super::register_and_broadcast(&registry, Some(&tx), "worker", entry).unwrap();
    assert!(
        registry
            .lock()
            .unwrap()
            .values()
            .any(|e| e.display_name == "worker")
    );
    let line = rx
        .try_recv()
        .expect("#866: spawn registration must broadcast immediately");
    assert!(line.ends_with('\n') && line.matches('\n').count() == 1); // #1055 framing
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    assert_eq!(v["subagents"][0]["agentId"], "worker");
    assert_eq!(v["subagents"][0]["status"], "starting");
}

#[test]
fn register_and_broadcast_without_channel_still_registers() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let entry = SubagentEntry::new(PathBuf::from("/tmp/x.sock"), 0);
    super::register_and_broadcast(&registry, None, "worker", entry).unwrap();
    assert!(
        registry
            .lock()
            .unwrap()
            .values()
            .any(|e| e.display_name == "worker")
    );
}

// ─── #1049: task-less spawn settles to Idle on registration ──────────────────
//
// Tests target `initial_registry_entry` (shared by production post-socket-ready
// registration and stub mode) so the status decision cannot drift between
// branches. Stub execute tests still exercise end-to-end registration+broadcast.

fn sample_config(task: Option<&str>) -> crate::domain::subagent::SubagentConfig {
    crate::domain::subagent::SubagentConfig {
        container: crate::domain::subagent::ContainerSelection::Local,
        task: task.map(String::from),
        agent_id: Some("worker".into()),
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: vec![],
        read_only: false,
    }
}

#[tokio::test]
async fn spawned_parent_id_ignores_empty_session_key_refresh() {
    use crate::domain::tool::Tool;

    let tool = SpawnTool::new(vec![], true)
        .with_event_forwarding(None, Some("existing-parent".to_string()));
    tool.set_session_key(String::new());
    let _ = tool
        .execute(r#"{"task":"work","agent_id":"child-empty-refresh"}"#)
        .await
        .unwrap();
    let registry = tool.registry().lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "child-empty-refresh")
        .expect("spawned entry should exist");
    assert_eq!(entry.parent_id.as_deref(), Some("existing-parent"));
}

#[tokio::test]
async fn spawned_parent_id_tracks_raw_session_key_changes() {
    use crate::domain::tool::Tool;

    let tool = SpawnTool::new(vec![], true).with_event_forwarding(None, Some("old".to_string()));
    tool.set_session_key("raw-session".to_string());
    let _ = tool
        .execute(r#"{"task":"work","agent_id":"child-raw"}"#)
        .await
        .unwrap();
    let registry = tool.registry().lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "child-raw")
        .expect("spawned entry should exist");
    assert_eq!(entry.parent_id.as_deref(), Some("raw-session"));
}

#[tokio::test]
async fn spawned_parent_id_tracks_cli_session_name_changes() {
    use crate::domain::tool::Tool;

    let tool = SpawnTool::new(vec![], true).with_event_forwarding(None, Some("old".to_string()));
    tool.set_session_key("cli:new-name".to_string());
    let _ = tool
        .execute(r#"{"task":"work","agent_id":"child-cli"}"#)
        .await
        .unwrap();
    let registry = tool.registry().lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "child-cli")
        .expect("spawned entry should exist");
    assert_eq!(entry.parent_id.as_deref(), Some("new-name"));
}

#[tokio::test]
async fn spawned_parent_id_tracks_colon_session_name_changes() {
    use crate::domain::tool::Tool;

    let tool = SpawnTool::new(vec![], true).with_event_forwarding(None, Some("old".to_string()));
    tool.set_session_key("kind:new-name".to_string());
    let _ = tool
        .execute(r#"{"task":"work","agent_id":"child-colon"}"#)
        .await
        .unwrap();
    let registry = tool.registry().lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "child-colon")
        .expect("spawned entry should exist");
    assert_eq!(entry.parent_id.as_deref(), Some("new-name"));
}

#[tokio::test]
async fn spawned_parent_id_tracks_session_key_changes() {
    use crate::domain::tool::Tool;

    let tool =
        SpawnTool::new(vec![], true).with_event_forwarding(None, Some("chat-old".to_string()));
    tool.set_session_key("chat-new".to_string());
    let _ = tool
        .execute(r#"{"task":"work","agent_id":"child-after-new"}"#)
        .await
        .unwrap();
    let registry = tool.registry().lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "child-after-new")
        .expect("spawned entry should exist");
    assert_eq!(entry.parent_id.as_deref(), Some("chat-new"));
}

#[test]
fn initial_entry_taskless_is_idle() {
    // Shared builder used by production after socket ready (#1049).
    let entry = super::initial_registry_entry(super::InitialRegistryEntrySpec {
        agent_uuid: crate::domain::ids::AgentUuid::new("00000000-0000-4000-8000-000000000101"),
        display_name: "ready".to_string(),
        socket_path: PathBuf::from("/tmp/ready.sock"),
        pid: 42,
        parent_id: Some("parent".into()),
        config: &sample_config(None),
        exit_signal_tx: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environment_registry: None,
        environment_ref: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
    });
    assert_eq!(entry.status, SubagentStatus::Idle);
    assert_eq!(entry.parent_id.as_deref(), Some("parent"));
    assert_eq!(entry.pid, 42);
    assert!(!entry.read_only);
}

#[test]
fn initial_entry_with_task_stays_starting() {
    let entry = super::initial_registry_entry(super::InitialRegistryEntrySpec {
        agent_uuid: crate::domain::ids::AgentUuid::new("00000000-0000-4000-8000-000000000102"),
        display_name: "ready".to_string(),
        socket_path: PathBuf::from("/tmp/ready.sock"),
        pid: 7,
        parent_id: None,
        config: &sample_config(Some("do work")),
        exit_signal_tx: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environment_registry: None,
        environment_ref: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
    });
    assert_eq!(
        entry.status,
        SubagentStatus::Starting,
        "#1049: with-task must stay Starting until agent_start"
    );
}

#[test]
fn initial_entry_taskless_broadcasts_idle_via_register() {
    // Prove the shared entry + register_and_broadcast path carries idle in the
    // snapshot (same event production emits after socket readiness).
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let entry = super::initial_registry_entry(super::InitialRegistryEntrySpec {
        agent_uuid: crate::domain::ids::AgentUuid::new("00000000-0000-4000-8000-000000000103"),
        display_name: "ready".to_string(),
        socket_path: PathBuf::from("/tmp/ready.sock"),
        pid: 1,
        parent_id: None,
        config: &sample_config(None),
        exit_signal_tx: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environment_registry: None,
        environment_ref: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
    });
    super::register_and_broadcast(&registry, Some(&tx), "idle-worker", entry).unwrap();
    let line = rx
        .try_recv()
        .expect("#1049: registration must broadcast state_changed");
    assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    assert_eq!(v["subagents"][0]["agentId"], "idle-worker");
    assert_eq!(
        v["subagents"][0]["status"], "idle",
        "#1049: broadcast must carry idle so cascade merges the child"
    );
}

#[tokio::test]
async fn taskless_stub_spawn_registers_as_idle() {
    // End-to-end stub path still uses the shared builder.
    let tool = SpawnTool::new(vec![], true);
    tool.execute(r#"{"agent_id":"idle-worker"}"#)
        .await
        .expect("stub spawn must succeed");
    let registry = tool.registry.lock().unwrap();
    let entry = registry
        .values()
        .find(|entry| entry.display_name == "idle-worker")
        .expect("task-less child must be registered");
    assert_eq!(
        entry.status,
        SubagentStatus::Idle,
        "#1049: task-less spawn must be Idle after registration"
    );
}

#[tokio::test]
async fn with_task_stub_spawn_stays_starting() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let tool = SpawnTool::new(vec![], true).with_event_forwarding(Some(tx), None);
    tool.execute(r#"{"task":"do work","agent_id":"busy-worker"}"#)
        .await
        .expect("stub spawn must succeed");
    {
        let registry = tool.registry.lock().unwrap();
        let entry = registry
            .values()
            .find(|entry| entry.display_name == "busy-worker")
            .expect("with-task child must be registered");
        assert_eq!(
            entry.status,
            SubagentStatus::Starting,
            "#1049: with-task spawn must stay Starting until agent_start"
        );
    }
    let line = rx
        .try_recv()
        .expect("with-task registration must still broadcast (#866)");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    assert_eq!(v["subagents"][0]["agentId"], "busy-worker");
    assert_eq!(
        v["subagents"][0]["status"], "starting",
        "#1049: with-task broadcast must carry starting, not idle"
    );
}

// ─── #1378: sockets / child session keys use AgentUuid, not display label ───

#[tokio::test]
async fn stub_spawn_keys_socket_by_uuid_not_display_label() {
    // Surviving adversarial finding on PR #1386: socket paths still used the
    // display label (`quecto-agent-reviewer.sock`), so respawning the same label
    // could collide with a stale socket or resume `cli:reviewer`.
    let tool = SpawnTool::new(vec![], true);
    tool.execute(r#"{"agent_id":"reviewer"}"#)
        .await
        .expect("stub spawn must succeed");

    let registry = tool.registry.lock().unwrap();
    let (key, entry) = registry
        .iter()
        .find(|(_, entry)| entry.display_name == "reviewer")
        .expect("reviewer must be registered");

    assert_eq!(
        key.as_str(),
        entry.agent_uuid.as_str(),
        "registry must be keyed by AgentUuid"
    );
    let sock = entry.socket_path.to_string_lossy();
    assert!(
        sock.contains(entry.agent_uuid.as_str()),
        "socket path must include AgentUuid, got {sock}"
    );
    assert!(
        !sock.contains("reviewer"),
        "socket path must not use the display label, got {sock}"
    );
    assert!(
        sock.ends_with(&format!("quecto-agent-{}.sock", entry.agent_uuid.as_str()))
            || sock.ends_with(&format!("/quecto-agent-{}.sock", entry.agent_uuid.as_str())),
        "expected quecto-agent-<uuid>.sock, got {sock}"
    );
}

#[tokio::test]
async fn stub_respawn_same_display_label_mints_fresh_uuid_and_socket() {
    let tool = SpawnTool::new(vec![], true);
    tool.execute(r#"{"agent_id":"reviewer"}"#)
        .await
        .expect("first spawn must succeed");
    {
        let mut registry = tool.registry.lock().unwrap();
        for entry in registry.values_mut() {
            if entry.display_name == "reviewer" {
                entry.status = SubagentStatus::Exited;
            }
        }
    }
    tool.execute(r#"{"agent_id":"reviewer"}"#)
        .await
        .expect("respawn after exit must succeed");

    let registry = tool.registry.lock().unwrap();
    let reviewers: Vec<_> = registry
        .values()
        .filter(|entry| entry.display_name == "reviewer")
        .collect();
    assert_eq!(reviewers.len(), 2, "both generations remain registered");
    assert_ne!(
        reviewers[0].agent_uuid, reviewers[1].agent_uuid,
        "each spawn must mint a fresh AgentUuid"
    );
    assert_ne!(
        reviewers[0].socket_path, reviewers[1].socket_path,
        "each spawn must get a distinct UUID-keyed socket path"
    );
    for entry in reviewers {
        let sock = entry.socket_path.to_string_lossy();
        assert!(
            sock.contains(entry.agent_uuid.as_str()),
            "socket must track its own UUID, got {sock}"
        );
        assert!(
            !sock.contains("reviewer"),
            "socket must not use display label"
        );
    }
}

#[test]
fn child_runtime_paths_are_uuid_keyed() {
    use crate::domain::ids::AgentUuid;
    let uuid = AgentUuid::new("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
    let dir = PathBuf::from("/run/user/1000");
    let socket = super::child_socket_path(&dir, &uuid);
    assert_eq!(
        socket,
        PathBuf::from("/run/user/1000/quecto-agent-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee.sock")
    );
    assert_eq!(
        super::child_session_key(&uuid),
        "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
    );
    assert_eq!(
        super::child_sidecar_filename("quecto-wfspec", &uuid, 4242),
        "quecto-wfspec-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee-4242.json"
    );
}

#[test]
fn test_definition_documents_container_spawning() {
    let tool = test_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let container = &schema["properties"]["container"];
    let desc = container["description"]
        .as_str()
        .expect("container documented");
    // Every accepted input form is discoverable from the schema alone.
    for needle in [
        "false",
        "true",
        "\"mode\":\"new\"",
        "\"mode\":\"existing\"",
        "container_config",
        "sandbox",
        "self-contained",
        "environment_ref=",
        "get_containers",
        "kill_container",
    ] {
        assert!(
            desc.contains(needle),
            "container description misses {needle}"
        );
    }
    // The absolute-path requirement AND the parent-config fallback are stated
    // where agents will read them (#1369 follow-up).
    let config_desc = schema["properties"]["config"]["description"]
        .as_str()
        .unwrap();
    assert!(config_desc.contains("container"));
    assert!(config_desc.contains("absolute"));
    assert!(config_desc.contains("falls back to the parent's own effective config path"));
    assert!(config_desc.contains("explicit config here wins"));
}

#[test]
fn test_definition_carries_the_container_config_roster() {
    // #1410: the tool description is the agent's session-start menu.
    let no_config = SpawnTool::new(vec![], true);
    assert!(
        no_config
            .definition()
            .description
            .contains("Available container configs: none configured."),
        "{}",
        no_config.definition().description
    );

    let dir = tempfile::TempDir::new().unwrap();
    let cfg = dir.path().join("config.json");
    std::fs::write(
        &cfg,
        r#"{"container_configs":{
            "quecto":{"default":true,"create":["/bin/true"],"cleanup":["/bin/true"]},
            "alpha":{"create":["/bin/true"],"cleanup":["/bin/true"]}}}"#,
    )
    .unwrap();
    let tool = SpawnTool::new(vec![], true).with_parent_config_path(Some(cfg));
    assert!(
        tool.definition()
            .description
            .contains("Available container configs: alpha, quecto (default)."),
        "{}",
        tool.definition().description
    );

    // A config that fails to load must degrade honestly, not panic.
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, "{not json").unwrap();
    let tool = SpawnTool::new(vec![], true).with_parent_config_path(Some(broken));
    assert!(
        tool.definition()
            .description
            .contains("Available container configs: unavailable (config failed to load)."),
        "{}",
        tool.definition().description
    );
}
