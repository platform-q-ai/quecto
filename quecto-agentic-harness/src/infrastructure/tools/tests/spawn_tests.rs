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
        .get("bound")
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
        .get("reviewer")
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
    assert!(tool.registry.lock().unwrap().contains_key("test"));
}
#[test]
fn register_and_broadcast_emits_immediate_state_changed() {
    // #866: spawn registration must broadcast the survivor set at once so a child
    // that begins a long first turn is visible in the TUI immediately, without
    // waiting for a GetSubagents poll or a terminal event.
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let entry = SubagentEntry::new(PathBuf::from("/tmp/x.sock"), 0);
    super::register_and_broadcast(&registry, Some(&tx), "worker", entry);
    assert!(registry.lock().unwrap().contains_key("worker"));
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
    super::register_and_broadcast(&registry, None, "worker", entry);
    assert!(registry.lock().unwrap().contains_key("worker"));
}
