use super::*;

#[test]
fn test_subagent_context_has_empty_history() {
    let config = SubagentConfig {
        task: Some("Do stuff".to_string()),
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    let ctx = SubagentContext::from_config(&config);
    assert_eq!(ctx.task, "Do stuff");
    assert!(ctx.messages.is_empty());
}

#[test]
fn test_subagent_inherits_restrict_true() {
    let config = SubagentConfig {
        task: Some("task".to_string()),
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    let ctx = SubagentContext::from_config(&config);
    assert!(ctx.restrict_to_workspace);
}

#[test]
fn test_subagent_inherits_restrict_false() {
    let config = SubagentConfig {
        task: Some("task".to_string()),
        agent_id: None,
        restrict_to_workspace: false,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    let ctx = SubagentContext::from_config(&config);
    assert!(!ctx.restrict_to_workspace);
}

#[test]
fn test_subagent_context_no_task() {
    let config = SubagentConfig {
        task: None,
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    let ctx = SubagentContext::from_config(&config);
    assert_eq!(ctx.task, "");
    assert!(ctx.messages.is_empty());
}
