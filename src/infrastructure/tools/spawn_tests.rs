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
fn test_validate_agent_id_format_empty_string() {
    let result = super::super::subagent_registry::validate_agent_id_format("");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("1-64 characters"));
}

#[test]
fn test_validate_agent_id_format_max_length_64() {
    let id = "a".repeat(64);
    let result = super::super::subagent_registry::validate_agent_id_format(&id);
    assert!(result.is_ok());
}

#[test]
fn test_validate_agent_id_format_too_long_65() {
    let id = "a".repeat(65);
    let result = super::super::subagent_registry::validate_agent_id_format(&id);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("1-64 characters"));
}

#[test]
fn test_validate_agent_id_format_all_valid_chars() {
    assert!(super::super::subagent_registry::validate_agent_id_format("abcXYZ019_-").is_ok());
}

#[test]
fn test_validate_agent_id_format_single_char() {
    use super::super::subagent_registry::validate_agent_id_format;
    assert!(validate_agent_id_format("a").is_ok());
    assert!(validate_agent_id_format("Z").is_ok());
    assert!(validate_agent_id_format("0").is_ok());
    assert!(validate_agent_id_format("_").is_ok());
    assert!(validate_agent_id_format("-").is_ok());
}

#[test]
fn test_validate_agent_id_format_invalid_dot() {
    let result = super::super::subagent_registry::validate_agent_id_format("hello.world");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
}

#[test]
fn test_validate_agent_id_format_invalid_space() {
    let result = super::super::subagent_registry::validate_agent_id_format("hello world");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
}

#[test]
fn test_validate_agent_id_format_invalid_slash() {
    let result = super::super::subagent_registry::validate_agent_id_format("a/b");
    assert!(result.is_err());
}

#[test]
fn test_validate_agent_id_format_invalid_unicode() {
    let result = super::super::subagent_registry::validate_agent_id_format("böt");
    assert!(result.is_err());
}
#[tokio::test]
async fn test_execute_stub_mode_success() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool
        .execute(r#"{"task":"Do something useful"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
}

#[tokio::test]
async fn test_execute_stub_mode_no_task() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.execute(r#"{"agent_id":"idle-worker"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
    assert!(result.content.contains("idle-worker"));
}

#[tokio::test]
async fn test_execute_stub_mode_registers_in_registry() {
    let tool = SpawnTool::new(vec![], true);
    let _result = tool
        .execute(r#"{"task":"work","agent_id":"my-bot"}"#)
        .await
        .unwrap();
    assert!(tool.registry.lock().unwrap().contains_key("my-bot"));
}

#[tokio::test]
async fn test_execute_stub_mode_default_agent_id() {
    let tool = SpawnTool::new(vec![], true);
    let _result = tool.execute(r#"{"task":"work"}"#).await.unwrap();
    assert!(tool.registry.lock().unwrap().contains_key("subagent"));
}

#[tokio::test]
async fn test_execute_stub_mode_with_agent_id() {
    let tool = SpawnTool::new(vec!["my-bot".to_string()], true);
    let result = tool
        .execute(r#"{"task":"fetch data","agent_id":"my-bot"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("my-bot"));
}
#[tokio::test]
async fn test_execute_invalid_json() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.execute("not valid json").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Failed to spawn subagent"));
    assert!(result.content.contains("invalid JSON"));
}

#[tokio::test]
async fn test_execute_disallowed_agent_returns_error() {
    let tool = SpawnTool::new(vec!["allowed-bot".to_string()], true);
    let result = tool
        .execute(r#"{"task":"evil","agent_id":"not-allowed"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not allowed"));
}

#[tokio::test]
async fn test_execute_invalid_agent_id_format_returns_error() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool
        .execute(r#"{"task":"test","agent_id":"bad id!"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("[a-zA-Z0-9_-]"));
}
#[test]
fn test_parse_args_invalid_json_garbage() {
    let tool = test_tool();
    let result = tool.parse_args("{garbage}}}");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid JSON"));
}

#[test]
fn test_parse_args_task_not_string() {
    let tool = test_tool();
    let config = tool.parse_args(r#"{"task":42}"#).unwrap();
    // task is not a string, so it's None
    assert!(config.task.is_none());
}

#[test]
fn test_parse_args_task_null() {
    let tool = test_tool();
    let config = tool.parse_args(r#"{"task":null}"#).unwrap();
    assert!(config.task.is_none());
}

#[test]
fn test_parse_args_system_not_string_ignored() {
    let tool = test_tool();
    let config = tool.parse_args(r#"{"task":"work","system":123}"#).unwrap();
    assert!(config.system.is_none());
}

#[test]
fn test_parse_args_agent_id_not_string_ignored() {
    let tool = test_tool();
    let config = tool
        .parse_args(r#"{"task":"work","agent_id":999}"#)
        .unwrap();
    assert!(config.agent_id.is_none());
}

#[test]
fn test_parse_args_restrict_to_workspace_inherited() {
    let tool_true = SpawnTool::new(vec![], true);
    let tool_false = SpawnTool::new(vec![], false);
    let cfg_t = tool_true.parse_args(r#"{"task":"a"}"#).unwrap();
    let cfg_f = tool_false.parse_args(r#"{"task":"a"}"#).unwrap();
    assert!(cfg_t.restrict_to_workspace);
    assert!(!cfg_f.restrict_to_workspace);
}
#[test]
fn test_shutdown_all_clears_registry() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    registry.lock().unwrap().insert(
        "bot".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
    );
    assert!(!registry.lock().unwrap().is_empty());
    shutdown_all(&registry);
    assert!(registry.lock().unwrap().is_empty());
}
#[test]
fn test_debug_trait() {
    let tool = SpawnTool::new(vec!["bot".to_string()], true);
    let debug_str = format!("{:?}", tool);
    assert!(debug_str.contains("SpawnTool"));
    assert!(debug_str.contains("bot"));
    assert!(debug_str.contains("restrict_to_workspace: true"));
}

#[test]
fn test_debug_with_base_dir() {
    let tool = SpawnTool::with_base_dir(vec![], false, PathBuf::from("/some/path"));
    let debug_str = format!("{:?}", tool);
    assert!(debug_str.contains("/some/path"));
}

// --- config_path validation ---

#[test]
fn test_effective_config_path_prefers_explicit_config() {
    let explicit = PathBuf::from("/tmp/explicit.json");
    let inherited = PathBuf::from("/tmp/inherited.json");

    assert_eq!(
        effective_config_path(Some(&explicit), Some(inherited)),
        Some(explicit)
    );
}

#[test]
fn test_effective_config_path_inherits_runtime_config_when_explicit_absent() {
    let inherited = PathBuf::from("/home/appuser/.quecto/runtime-configs/run.json");

    assert_eq!(
        effective_config_path(None, Some(inherited.clone())),
        Some(inherited)
    );
}

#[test]
fn test_parse_config_path_valid_absolute() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"work","config":"/home/user/.quecto/config.json"}"#)
        .unwrap();
    assert_eq!(
        cfg.config_path,
        Some(PathBuf::from("/home/user/.quecto/config.json"))
    );
}

#[test]
fn test_parse_config_path_valid_relative() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"work","config":"configs/custom.json"}"#)
        .unwrap();
    assert_eq!(cfg.config_path, Some(PathBuf::from("configs/custom.json")));
}

#[test]
fn test_parse_config_path_traversal_rejected() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.parse_args(r#"{"task":"work","config":"../../etc/shadow"}"#);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains(".."), "expected '..' in error, got: {err}");
    assert!(
        err.contains("not allowed"),
        "expected 'not allowed' in error, got: {err}"
    );
}

#[test]
fn test_parse_config_path_traversal_absolute_rejected() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.parse_args(r#"{"task":"work","config":"/safe/../etc/shadow"}"#);
    assert!(result.is_err());
}

#[test]
fn test_parse_config_path_absent_is_none() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
    assert!(cfg.config_path.is_none());
}

#[test]
fn test_parse_config_path_non_string_ignored() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool.parse_args(r#"{"task":"work","config":123}"#).unwrap();
    assert!(cfg.config_path.is_none());
}

// --- workflow / workflow_guards validation ---

#[test]
fn test_parse_workflow_true() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow":true}"#)
        .unwrap();
    assert!(cfg.workflow);
    assert!(!cfg.workflow_guards);
}

#[test]
fn test_parse_workflow_false_by_default() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
    assert!(!cfg.workflow);
    assert!(!cfg.workflow_guards);
}

#[test]
fn test_parse_workflow_guards_requires_workflow() {
    let tool = SpawnTool::new(vec![], true);
    let result = tool.parse_args(r#"{"task":"work","workflow_guards":true}"#);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("workflow_guards requires workflow")
    );
}

#[test]
fn test_parse_workflow_guards_with_workflow_ok() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow":true,"workflow_guards":true}"#)
        .unwrap();
    assert!(cfg.workflow);
    assert!(cfg.workflow_guards);
}

#[test]
fn test_parse_workflow_non_bool_ignored() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow":"yes"}"#)
        .unwrap();
    assert!(!cfg.workflow);
}

#[test]
fn test_parse_workflow_guards_non_bool_ignored() {
    let tool = SpawnTool::new(vec![], true);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow_guards":1}"#)
        .unwrap();
    assert!(!cfg.workflow_guards);
}

// --- validate_config_path unit tests ---

#[test]
fn test_validate_config_path_clean_absolute() {
    let result = validate_config_path("/home/user/.quecto/config.json");
    assert!(result.is_ok());
}

#[test]
fn test_validate_config_path_clean_relative() {
    let result = validate_config_path("configs/custom.json");
    assert!(result.is_ok());
}

#[test]
fn test_validate_config_path_dotdot_relative() {
    let result = validate_config_path("../../etc/shadow");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains(".."));
    assert!(err.contains("not allowed"));
}

#[test]
fn test_validate_config_path_dotdot_embedded() {
    let result = validate_config_path("/safe/path/../etc/passwd");
    assert!(result.is_err());
}

#[test]
fn test_validate_config_path_single_dot_ok() {
    // A single "." (current dir) is fine — it's not traversal.
    let result = validate_config_path("./config.json");
    assert!(result.is_ok());
}
