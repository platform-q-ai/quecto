use super::*;

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
    let tool = SpawnTool::new(vec![]);
    let result = tool
        .execute(r#"{"task":"Do something useful"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
}

#[tokio::test]
async fn test_execute_stub_mode_no_task() {
    let tool = SpawnTool::new(vec![]);
    let result = tool.execute(r#"{"agent_id":"idle-worker"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
    assert!(result.content.contains("idle-worker"));
}

#[tokio::test]
async fn test_execute_stub_mode_registers_in_registry() {
    let tool = SpawnTool::new(vec![]);
    let _result = tool
        .execute(r#"{"task":"work","agent_id":"my-bot"}"#)
        .await
        .unwrap();
    assert!(tool.registry.lock().unwrap().contains_key("my-bot"));
}

#[tokio::test]
async fn test_spawned_entry_carries_parent_id() {
    // Regression (#820 panel tree): a spawned child's registry entry must record
    // the spawning agent's own id as its parent_id, otherwise grandchildren can
    // never nest under their real parent in the sub-agent panel.
    let tool = SpawnTool::new(vec![]).with_event_forwarding(None, Some("childA".to_string()));
    let _ = tool
        .execute(r#"{"task":"work","agent_id":"grandchildB"}"#)
        .await
        .unwrap();
    let registry = tool.registry.lock().unwrap();
    let entry = registry
        .get("grandchildB")
        .expect("spawned entry should exist");
    assert_eq!(entry.parent_id.as_deref(), Some("childA"));
}

#[tokio::test]
async fn test_execute_stub_mode_default_agent_id() {
    let tool = SpawnTool::new(vec![]);
    let _result = tool.execute(r#"{"task":"work"}"#).await.unwrap();
    assert!(tool.registry.lock().unwrap().contains_key("subagent"));
}

#[tokio::test]
async fn test_execute_stub_mode_with_agent_id() {
    let tool = SpawnTool::new(vec!["my-bot".to_string()]);
    let result = tool
        .execute(r#"{"task":"fetch data","agent_id":"my-bot"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("my-bot"));
}
#[tokio::test]
async fn test_execute_invalid_json() {
    let tool = SpawnTool::new(vec![]);
    let result = tool.execute("not valid json").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Failed to spawn subagent"));
    assert!(result.content.contains("invalid JSON"));
}

#[tokio::test]
async fn test_execute_disallowed_agent_returns_error() {
    let tool = SpawnTool::new(vec!["allowed-bot".to_string()]);
    let result = tool
        .execute(r#"{"task":"evil","agent_id":"not-allowed"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not allowed"));
}

#[tokio::test]
async fn test_execute_invalid_agent_id_format_returns_error() {
    let tool = SpawnTool::new(vec![]);
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
    let tool_true = SpawnTool::new(vec![]);
    let tool_false = SpawnTool::new(vec![]);
    let cfg_t = tool_true.parse_args(r#"{"task":"a"}"#).unwrap();
    let cfg_f = tool_false.parse_args(r#"{"task":"a"}"#).unwrap();
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
    let tool = SpawnTool::new(vec!["bot".to_string()]);
    let debug_str = format!("{:?}", tool);
    assert!(debug_str.contains("SpawnTool"));
    assert!(debug_str.contains("bot"));
}

#[test]
fn test_debug_with_base_dir() {
    let tool = SpawnTool::with_base_dir(vec![], PathBuf::from("/some/path"));
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
    let tool = SpawnTool::new(vec![]);
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
    let tool = SpawnTool::new(vec![]);
    let cfg = tool
        .parse_args(r#"{"task":"work","config":"configs/custom.json"}"#)
        .unwrap();
    assert_eq!(cfg.config_path, Some(PathBuf::from("configs/custom.json")));
}

#[test]
fn test_parse_config_path_traversal_rejected() {
    let tool = SpawnTool::new(vec![]);
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
    let tool = SpawnTool::new(vec![]);
    let result = tool.parse_args(r#"{"task":"work","config":"/safe/../etc/shadow"}"#);
    assert!(result.is_err());
}

#[test]
fn test_parse_config_path_absent_is_none() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
    assert!(cfg.config_path.is_none());
}

#[test]
fn test_parse_config_path_non_string_ignored() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool.parse_args(r#"{"task":"work","config":123}"#).unwrap();
    assert!(cfg.config_path.is_none());
}

// --- workflow / workflow_guards validation ---

#[test]
fn test_parse_workflow_true() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow":true}"#)
        .unwrap();
    assert!(cfg.workflow);
    assert!(!cfg.workflow_guards);
}

#[test]
fn test_parse_workflow_false_by_default() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
    assert!(!cfg.workflow);
    assert!(!cfg.workflow_guards);
}

#[test]
fn test_parse_workflow_guards_requires_workflow() {
    let tool = SpawnTool::new(vec![]);
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
    let tool = SpawnTool::new(vec![]);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow":true,"workflow_guards":true}"#)
        .unwrap();
    assert!(cfg.workflow);
    assert!(cfg.workflow_guards);
}

#[test]
fn test_parse_workflow_non_bool_ignored() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool
        .parse_args(r#"{"task":"work","workflow":"yes"}"#)
        .unwrap();
    assert!(!cfg.workflow);
}

#[test]
fn test_parse_workflow_guards_non_bool_ignored() {
    let tool = SpawnTool::new(vec![]);
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

#[test]
fn with_event_forwarding_sets_broadcast_and_parent() {
    let (tx, _rx) = tokio::sync::broadcast::channel::<String>(4);
    let tool =
        SpawnTool::new(vec![]).with_event_forwarding(Some(tx), Some("root".to_string()));
    // Fields are private; the Debug projection exercises the builder + fields.
    assert!(format!("{tool:?}").contains("root"));
}

#[tokio::test]
async fn execute_in_stub_mode_returns_running_message() {
    // SpawnTool::new has an empty base_dir → execute() runs in stub mode and
    // does not spawn a real process.
    let tool = SpawnTool::new(vec![]);
    let result = tool
        .execute(r#"{"task":"do it","agent_id":"probe"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "stub execute should succeed: {}",
        result.content
    );
    assert!(result.content.contains("probe"));
}

#[tokio::test]
async fn execute_with_invalid_args_is_error() {
    let tool = SpawnTool::new(vec![]);
    let result = tool
        .execute(r#"{"workflow_spec":{"no":"template"}}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid workflow_spec"));
}

// --- model passthrough (#881) ---

#[test]
fn test_parse_model_full_string() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool
        .parse_args(r#"{"task":"work","model":"openai/gpt-5.5"}"#)
        .unwrap();
    assert_eq!(cfg.model.as_deref(), Some("openai/gpt-5.5"));
}

#[test]
fn test_parse_model_provider_and_model_id() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool
        .parse_args(r#"{"task":"work","provider":"openai","model_id":"gpt-5.5"}"#)
        .unwrap();
    assert_eq!(cfg.model.as_deref(), Some("openai/gpt-5.5"));
}

#[test]
fn test_parse_model_absent_is_none() {
    let tool = SpawnTool::new(vec![]);
    let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
    assert!(cfg.model.is_none());
}

#[test]
fn test_parse_model_provider_without_model_id_is_error() {
    let tool = SpawnTool::new(vec![]);
    let err = tool
        .parse_args(r#"{"task":"work","provider":"openai"}"#)
        .unwrap_err();
    assert!(err.contains("invalid model"), "got: {err}");
    assert!(err.contains("model_id"), "got: {err}");
}

#[tokio::test]
async fn execute_with_invalid_model_is_error() {
    let tool = SpawnTool::new(vec![]);
    let result = tool
        .execute(r#"{"task":"work","provider":"openai"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid model"));
}

#[test]
fn test_schema_includes_model_property() {
    let tool = SpawnTool::new(vec![]);
    let schema: serde_json::Value =
        serde_json::from_str(&tool.definition().parameters_schema).unwrap();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("model"));
    assert!(props.contains_key("provider"));
    assert!(props.contains_key("model_id"));
}
