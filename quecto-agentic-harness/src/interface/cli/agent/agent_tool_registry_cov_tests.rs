use super::*;
use crate::interface::cli::agent::flag_parse::AgentFlags;

fn flags() -> AgentFlags {
    AgentFlags {
        session_name: Some("dev".to_string()),
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: false,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: Vec::new(),
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: true,
        workflow_spec_path: None,
        parent_id: None,
        spawned: false,
    }
}

#[test]
fn resolve_agent_model_prefers_explicit_override() {
    assert_eq!(
        resolve_agent_model(Some("openai-api/gpt-5.6-sol"), "anthropic-api/claude"),
        "openai-api/gpt-5.6-sol"
    );
}

#[test]
fn resolve_agent_model_falls_back_to_config_default() {
    assert_eq!(
        resolve_agent_model(None, "anthropic-api/claude-sonnet-4.5"),
        "anthropic-api/claude-sonnet-4.5"
    );
}

#[test]
fn build_tool_registry_warns_when_sandbox_disabled_and_uses_empty_session_for_no_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();
    let mut flags = flags();
    flags.no_sandbox = true;
    flags.no_session = true;
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    assert!(
        stderr.contains("--no-sandbox is active"),
        "stderr: {stderr}"
    );
    assert_eq!(built.session_key, "");
    assert_eq!(built.model, config.agents.defaults.model);
    assert!(built.notification_rx.is_some());
    assert!(built.subagent_registry.is_some());
}

#[test]
fn build_tool_registry_uses_cli_session_name_and_model_override() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();
    let http = reqwest::Client::new();
    let mut flags = flags();
    flags.session_name = Some("named".to_string());
    flags.model_override = Some("openai-api/gpt-5.6-sol".to_string());
    let mut stderr = String::new();

    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config: &config,
        http_client: &http,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
    })
    .unwrap();

    assert_eq!(built.session_key, Session::build_key("cli", "named"));
    assert_eq!(built.model, "openai-api/gpt-5.6-sol");
    assert!(!built.extension_prompt_snippets.contains("failed"));
}

#[test]
fn load_workflow_spec_reports_io_and_json_errors_and_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let missing = tmp.path().join("missing.json");
    let missing_err = load_workflow_spec(&missing).unwrap_err();
    assert!(
        missing_err.contains("No such") || missing_err.contains("os error"),
        "{missing_err}"
    );

    let bad = tmp.path().join("bad.json");
    std::fs::write(&bad, "not-json").unwrap();
    let json_err = load_workflow_spec(&bad).unwrap_err();
    assert!(
        json_err.contains("expected") || json_err.contains("invalid"),
        "{json_err}"
    );

    let good = tmp.path().join("good.json");
    std::fs::write(
        &good,
        serde_json::json!({
            "template": {
                "id": "cov-template",
                "label": "Coverage Template",
                "description": "test workflow spec",
                "steps": [
                    {"key": "one", "label": "One", "phase": "Act"}
                ]
            }
        })
        .to_string(),
    )
    .unwrap();
    let spec = load_workflow_spec(&good).unwrap();
    assert_eq!(spec.template.id, "cov-template");
    assert_eq!(spec.template.steps.len(), 1);
}

#[test]
fn load_workflow_spec_reports_missing_unreadable_and_malformed_specs() {
    let dir = tempfile::tempdir().expect("tempdir");

    // 1. Absent file: metadata fails before anything is read.
    let missing = dir.path().join("absent.json");
    let err = load_workflow_spec(&missing).expect_err("an absent spec must fail");
    assert!(!err.is_empty(), "error message should not be empty");

    // 2. A directory in place of the spec: metadata succeeds, the read fails.
    let as_dir = dir.path().join("spec-dir.json");
    std::fs::create_dir(&as_dir).expect("create dir");
    load_workflow_spec(&as_dir).expect_err("a directory is not a readable spec");

    // 3. Well-formed file, malformed JSON.
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, b"{ not a spec").expect("write malformed spec");
    let err = load_workflow_spec(&bad).expect_err("malformed JSON must fail");
    assert!(
        err.contains("expected") || err.contains("key") || err.contains("column"),
        "expected a serde parse message, got: {err}"
    );
}

#[test]
fn load_workflow_spec_consumes_the_file_on_success() {
    use crate::domain::workflow::{WorkflowSpec, WorkflowTemplate, WorkflowTemplateStep};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("spec.json");
    let spec = WorkflowSpec {
        template: WorkflowTemplate {
            id: "t1".into(),
            label: "T1".into(),
            description: "d".into(),
            when_to_use: None,
            steps: vec![WorkflowTemplateStep {
                key: "s".into(),
                label: "S".into(),
                phase: "p".into(),
                guidance: None,
            }],
            guards: vec![],
        },
    };
    std::fs::write(&path, serde_json::to_string(&spec).unwrap()).expect("write spec");

    let loaded = load_workflow_spec(&path).expect("a well-formed spec loads");
    assert_eq!(loaded.template.id, "t1");
    // Single-use: the spec must not linger beside the socket after being read.
    assert!(!path.exists(), "spec file was not consumed after loading");
}
